package main

import (
	"archive/zip"
	"bufio"
	"bytes"
	"context"
	"embed"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"syscall"
	"time"
)

//go:embed web/*
var embeddedWebFS embed.FS

// AppState holds the global state
type AppState struct {
	mu            sync.RWMutex
	cmd           *exec.Cmd
	isRunning     bool
	startTime     time.Time
	config        Config
	logBuffer     []string
	logClients    map[chan string]bool
	downloadSub   map[chan DownloadProgress]bool
	activeTarget  string
	qemuDir       string
	appDir        string
}

type Config struct {
	Host            string `json:"host"`
	Port            int    `json:"port"`
	Proxy           string `json:"proxy"`
	Engine          string `json:"engine"`
	Memory          string `json:"memory"`
	SMP             string `json:"smp"`
	Accel           string `json:"accel"`
	LogLevel        string `json:"logLevel"`
	RefreshInterval string `json:"refreshInterval"`
}

type LoginRequest struct {
	Username   string `json:"username"`
	Password   string `json:"password"`
	TwoFactor  string `json:"twoFactor"`
	DeviceInfo string `json:"deviceInfo"`
	Proxy      string `json:"proxy"`
}

type DownloadProgress struct {
	Status     string  `json:"status"`
	Percent    float64 `json:"percent"`
	Downloaded int64   `json:"downloaded"`
	Total      int64   `json:"total"`
	Speed      int64   `json:"speed"`
	Message    string  `json:"message,omitempty"`
}

var state = &AppState{
	logClients:  make(map[chan string]bool),
	downloadSub: make(map[chan DownloadProgress]bool),
	config: Config{
		Host:            "127.0.0.1",
		Port:            12340,
		Engine:          "qemu",
		Memory:          "512",
		SMP:             "2",
		Accel:           "auto",
		LogLevel:        "info",
		RefreshInterval: "1800",
	},
}

func main() {
	guiPort := flag.Int("port", 12341, "Port for wrapper-lite GUI")
	guiHost := flag.String("host", "127.0.0.1", "Host binding for GUI")
	noBrowser := flag.Bool("no-browser", false, "Do not auto-open browser")
	flag.Parse()

	exePath, err := os.Executable()
	if err == nil {
		state.appDir = filepath.Dir(exePath)
	} else {
		state.appDir, _ = os.Getwd()
	}
	state.qemuDir = filepath.Join(state.appDir, "qemu")

	mux := http.NewServeMux()

	// REST APIs
	mux.HandleFunc("/api/info", handleInfo)
	mux.HandleFunc("/api/status", handleStatus)
	mux.HandleFunc("/api/start", handleStart)
	mux.HandleFunc("/api/stop", handleStop)
	mux.HandleFunc("/api/login", handleLogin)
	mux.HandleFunc("/api/qemu/status", handleQemuStatus)
	mux.HandleFunc("/api/qemu/download", handleQemuDownload)
	mux.HandleFunc("/api/qemu/download/progress", handleDownloadProgressSSE)
	mux.HandleFunc("/api/logs", handleGetLogs)
	mux.HandleFunc("/api/logs/stream", handleLogsSSE)

	// Static Web Frontend (served from embedded files)
	subFS, err := fs.Sub(embeddedWebFS, "web")
	if err != nil {
		log.Fatalf("Failed to initialize embedded web filesystem: %v", err)
	}
	fileServer := http.FileServer(http.FS(subFS))
	mux.Handle("/", fileServer)

	addr := fmt.Sprintf("%s:%d", *guiHost, *guiPort)
	listener, err := net.Listen("tcp", addr)
	if err != nil {
		// Try next available port
		addr = fmt.Sprintf("%s:0", *guiHost)
		listener, err = net.Listen("tcp", addr)
		if err != nil {
			log.Fatalf("Failed to bind GUI port: %v", err)
		}
	}

	actualPort := listener.Addr().(*net.TCPAddr).Port
	url := fmt.Sprintf("http://127.0.0.1:%d", actualPort)
	fmt.Printf("====================================================\n")
	fmt.Printf("  wrapper-lite Cross-Platform GUI is running!\n")
	fmt.Printf("  URL: %s\n", url)
	fmt.Printf("  OS: %s | Arch: %s\n", runtime.GOOS, runtime.GOARCH)
	fmt.Printf("====================================================\n")

	// Open browser unless disabled
	if !*noBrowser {
		go func() {
			time.Sleep(350 * time.Millisecond)
			openBrowser(url)
		}()
	}

	// Graceful shutdown handling
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-sigChan
		fmt.Println("\nShutting down GUI...")
		state.stopService()
		os.Exit(0)
	}()

	if err := http.Serve(listener, mux); err != nil && err != http.ErrServerClosed {
		log.Fatalf("Server error: %v", err)
	}
}

func handleInfo(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{
		"os":         runtime.GOOS,
		"arch":       runtime.GOARCH,
		"nightlyUrl": getNightlyUrl(),
		"appDir":     state.appDir,
	})
}

func handleStatus(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	state.mu.RLock()
	defer state.mu.RUnlock()

	uptime := float64(0)
	if state.isRunning && !state.startTime.IsZero() {
		uptime = time.Since(state.startTime).Seconds()
	}

	json.NewEncoder(w).Encode(map[string]interface{}{
		"running": state.isRunning,
		"uptime":  uptime,
		"host":    state.config.Host,
		"port":    state.config.Port,
		"engine":  state.config.Engine,
	})
}

func handleStart(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	var cfg Config
	if err := json.NewDecoder(r.Body).Decode(&cfg); err != nil {
		cfg = state.config
	}

	err := state.startService(cfg)
	w.Header().Set("Content-Type", "application/json")
	if err != nil {
		w.WriteHeader(http.StatusBadRequest)
		json.NewEncoder(w).Encode(map[string]interface{}{
			"success": false,
			"message": err.Error(),
		})
		return
	}

	json.NewEncoder(w).Encode(map[string]interface{}{
		"success": true,
	})
}

func handleStop(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	state.stopService()
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{"success": true})
}

func handleLogin(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	var req LoginRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	res, err := state.performLogin(req)
	w.Header().Set("Content-Type", "application/json")
	if err != nil {
		w.WriteHeader(http.StatusBadRequest)
		json.NewEncoder(w).Encode(map[string]interface{}{
			"success": false,
			"message": err.Error(),
		})
		return
	}

	json.NewEncoder(w).Encode(res)
}

func handleQemuStatus(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	st := state.checkQemuFiles()
	json.NewEncoder(w).Encode(st)
}

func handleQemuDownload(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	go state.downloadQemuPackage()
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{"started": true})
}

func handleDownloadProgressSSE(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")

	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "Streaming unsupported", http.StatusInternalServerError)
		return
	}

	ch := make(chan DownloadProgress, 10)
	state.mu.Lock()
	state.downloadSub[ch] = true
	state.mu.Unlock()

	defer func() {
		state.mu.Lock()
		delete(state.downloadSub, ch)
		state.mu.Unlock()
	}()

	for {
		select {
		case <-r.Context().Done():
			return
		case p := <-ch:
			data, _ := json.Marshal(p)
			fmt.Fprintf(w, "data: %s\n\n", data)
			flusher.Flush()
			if p.Status == "done" || p.Status == "error" {
				return
			}
		}
	}
}

func handleGetLogs(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	state.mu.RLock()
	defer state.mu.RUnlock()
	logs := state.logBuffer
	if logs == nil {
		logs = []string{}
	}
	json.NewEncoder(w).Encode(logs)
}

func handleLogsSSE(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")

	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "Streaming unsupported", http.StatusInternalServerError)
		return
	}

	ch := make(chan string, 50)
	state.mu.Lock()
	state.logClients[ch] = true
	// Send existing buffer first
	for _, l := range state.logBuffer {
		fmt.Fprintf(w, "data: %s\n\n", l)
	}
	flusher.Flush()
	state.mu.Unlock()

	defer func() {
		state.mu.Lock()
		delete(state.logClients, ch)
		state.mu.Unlock()
	}()

	for {
		select {
		case <-r.Context().Done():
			return
		case line := <-ch:
			fmt.Fprintf(w, "data: %s\n\n", line)
			flusher.Flush()
		}
	}
}

// Service Implementation
func (s *AppState) startService(cfg Config) error {
	s.mu.Lock()
	if s.isRunning {
		s.mu.Unlock()
		return fmt.Errorf("service is already running")
	}
	s.config = cfg
	s.mu.Unlock()

	var bin string
	var args []string

	// Non-Linux or when engine=qemu is selected
	if cfg.Engine == "qemu" || runtime.GOOS != "linux" {
		bin = s.findLauncher("wrapper-lite-qemu")
		if bin == "" {
			return fmt.Errorf("wrapper-lite-qemu not found. Please download the QEMU package from the 'QEMU Package' tab")
		}

		args = []string{
			"--host", cfg.Host,
			"--host-port", fmt.Sprintf("%d", cfg.Port),
			"--guest-port", "12340",
			"--guest-host", "0.0.0.0",
			"--memory", cfg.Memory,
			"--smp", cfg.SMP,
		}

		if cfg.Accel != "" && cfg.Accel != "auto" {
			args = append(args, "--accel", cfg.Accel)
		}

		if cfg.Proxy != "" {
			args = append(args, "--proxy", cfg.Proxy)
		}
		if cfg.LogLevel != "" {
			args = append(args, "--log-level", cfg.LogLevel)
		}
		if cfg.RefreshInterval != "" {
			args = append(args, "--token-refresh-interval", cfg.RefreshInterval)
		}
	} else {
		// Native mode on Linux
		bin = s.findLauncher("wrapper-lite-rootless")
		if bin == "" {
			bin = s.findLauncher("wrapper-lite")
		}
		if bin == "" {
			return fmt.Errorf("native wrapper-lite binary not found")
		}

		args = []string{
			"--base-dir", filepath.Join(s.appDir, "rootfs", "data"),
			"--host", cfg.Host,
			"--port", fmt.Sprintf("%d", cfg.Port),
		}
		if cfg.Proxy != "" {
			args = append(args, "--proxy", cfg.Proxy)
		}
		if cfg.LogLevel != "" {
			args = append(args, "--log-level", cfg.LogLevel)
		}
		if cfg.RefreshInterval != "" {
			args = append(args, "--token-refresh-interval", cfg.RefreshInterval)
		}
	}

	cmd := exec.Command(bin, args...)
	cmd.Dir = s.appDir
	setProcessGroup(cmd)

	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return err
	}

	if err := cmd.Start(); err != nil {
		return fmt.Errorf("failed to start process %s: %w", bin, err)
	}

	s.mu.Lock()
	s.cmd = cmd
	s.isRunning = true
	s.startTime = time.Now()
	s.mu.Unlock()

	s.appendLog(fmt.Sprintf("[run] Process started (PID %d): %s %s", cmd.Process.Pid, bin, strings.Join(args, " ")))

	// Stream stdout & stderr
	go s.streamOutput(stdout)
	go s.streamOutput(stderr)

	go func() {
		err := cmd.Wait()
		s.mu.Lock()
		s.isRunning = false
		s.cmd = nil
		s.mu.Unlock()
		if err != nil {
			s.appendLog(fmt.Sprintf("[run] Process exited with error: %v", err))
		} else {
			s.appendLog("[run] Process exited normally.")
		}
	}()

	return nil
}

func (s *AppState) stopService() {
	s.mu.Lock()
	defer s.mu.Unlock()

	if !s.isRunning || s.cmd == nil || s.cmd.Process == nil {
		return
	}

	s.appendLog(fmt.Sprintf("[run] Terminating process (PID %d)...", s.cmd.Process.Pid))
	killProcessTree(s.cmd)

	s.isRunning = false
	s.cmd = nil
}

func (s *AppState) performLogin(req LoginRequest) (map[string]interface{}, error) {
	bin := s.findLauncher("wrapper-lite-qemu")
	isQemu := true
	if bin == "" && runtime.GOOS == "linux" {
		bin = s.findLauncher("wrapper-lite-rootless")
		if bin == "" {
			bin = s.findLauncher("wrapper-lite")
		}
		isQemu = false
	}
	if bin == "" {
		return nil, fmt.Errorf("neither wrapper-lite-qemu nor wrapper-lite found")
	}

	fullPassword := req.Password
	if req.TwoFactor != "" {
		fullPassword += req.TwoFactor
	}

	loginCred := fmt.Sprintf("%s:%s", req.Username, fullPassword)
	var args []string

	if isQemu {
		args = []string{
			"--login", loginCred,
			"--code-from-file",
		}
		if req.Proxy != "" {
			args = append(args, "--proxy", req.Proxy)
		}
		if req.DeviceInfo != "" {
			args = append(args, "--device-info", req.DeviceInfo)
		}
	} else {
		args = []string{
			"--login", loginCred,
			"--code-from-file",
			"--base-dir", filepath.Join(s.appDir, "rootfs", "data"),
		}
		if req.Proxy != "" {
			args = append(args, "--proxy", req.Proxy)
		}
		if req.DeviceInfo != "" {
			args = append(args, "--device-info", req.DeviceInfo)
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 120*time.Second)
	defer cancel()

	cmd := exec.CommandContext(ctx, bin, args...)
	cmd.Dir = s.appDir

	var outBuf bytes.Buffer
	cmd.Stdout = io.MultiWriter(&outBuf, os.Stdout)
	cmd.Stderr = io.MultiWriter(&outBuf, os.Stderr)

	s.appendLog(fmt.Sprintf("[auth] Running login command for %s...", req.Username))

	err := cmd.Run()
	outputStr := outBuf.String()

	// Parse lines for logging
	lines := strings.Split(outputStr, "\n")
	for _, l := range lines {
		if strings.TrimSpace(l) != "" {
			s.appendLog(l)
		}
	}

	if err != nil {
		if strings.Contains(outputStr, "need2FA: true") || strings.Contains(outputStr, "2FA code") {
			return map[string]interface{}{
				"success": false,
				"need2FA": true,
				"message": "Two-factor authentication required",
			}, nil
		}
		return map[string]interface{}{
			"success": false,
			"message": fmt.Sprintf("Login failed: %v", err),
		}, nil
	}

	return map[string]interface{}{
		"success": true,
		"message": "Login succeeded! Tokens cached.",
	}, nil
}

func (s *AppState) streamOutput(r io.Reader) {
	scanner := bufio.NewScanner(r)
	for scanner.Scan() {
		line := scanner.Text()
		s.appendLog(line)
	}
}

func (s *AppState) appendLog(line string) {
	s.mu.Lock()
	defer s.mu.Unlock()

	s.logBuffer = append(s.logBuffer, line)
	if len(s.logBuffer) > 2000 {
		s.logBuffer = s.logBuffer[len(s.logBuffer)-2000:]
	}

	for ch := range s.logClients {
		select {
		case ch <- line:
		default:
		}
	}
}

func (s *AppState) broadcastDownload(p DownloadProgress) {
	s.mu.Lock()
	defer s.mu.Unlock()

	for ch := range s.downloadSub {
		select {
		case ch <- p:
		default:
		}
	}
}

func (s *AppState) findLauncher(name string) string {
	ext := ""
	if runtime.GOOS == "windows" {
		ext = ".exe"
	}
	target := name + ext

	// 1. Current working directory or app directory
	candidates := []string{
		filepath.Join(s.appDir, target),
		filepath.Join(".", target),
		filepath.Join(s.appDir, "..", target),
	}

	for _, c := range candidates {
		if fi, err := os.Stat(c); err == nil && !fi.IsDir() {
			return c
		}
	}

	// 2. PATH
	if p, err := exec.LookPath(target); err == nil {
		return p
	}

	return ""
}

type QemuCheckResult struct {
	Launcher   bool `json:"launcher"`
	Kernel     bool `json:"kernel"`
	Initramfs  bool `json:"initramfs"`
	Disk       bool `json:"disk"`
	QemuBin    bool `json:"qemuBin"`
	AllPresent bool `json:"allPresent"`
}

func (s *AppState) checkQemuFiles() QemuCheckResult {
	launcher := s.findLauncher("wrapper-lite-qemu") != ""

	kernelCandidates := []string{
		filepath.Join(s.qemuDir, "vmlinuz-lite-qemu"),
		filepath.Join(".", "qemu", "vmlinuz-lite-qemu"),
	}
	kernel := fileExistsAny(kernelCandidates)

	initrdCandidates := []string{
		filepath.Join(s.qemuDir, "lite-initramfs.cpio.gz"),
		filepath.Join(".", "qemu", "lite-initramfs.cpio.gz"),
	}
	initramfs := fileExistsAny(initrdCandidates)

	diskCandidates := []string{
		filepath.Join(s.qemuDir, "data.img"),
		filepath.Join(".", "qemu", "data.img"),
	}
	disk := fileExistsAny(diskCandidates)

	qemuExe := "qemu-system-x86_64"
	if runtime.GOOS == "windows" {
		qemuExe += ".exe"
	}
	qemuBinCandidates := []string{
		filepath.Join(s.qemuDir, "bin", qemuExe),
		filepath.Join(".", "qemu", "bin", qemuExe),
	}
	if runtime.GOOS == "windows" {
		qemuBinCandidates = append(qemuBinCandidates,
			`C:\Program Files\qemu\`+qemuExe,
			`C:\ProgramData\chocolatey\bin\`+qemuExe,
		)
	} else if runtime.GOOS == "darwin" {
		qemuBinCandidates = append(qemuBinCandidates,
			"/opt/homebrew/bin/"+qemuExe,
			"/usr/local/bin/"+qemuExe,
		)
	} else {
		qemuBinCandidates = append(qemuBinCandidates,
			"/usr/bin/"+qemuExe,
			"/usr/local/bin/"+qemuExe,
		)
	}
	qemuBin := fileExistsAny(qemuBinCandidates)
	if !qemuBin {
		// Also check PATH
		if _, err := exec.LookPath(qemuExe); err == nil {
			qemuBin = true
		}
	}

	return QemuCheckResult{
		Launcher:   launcher,
		Kernel:     kernel,
		Initramfs:  initramfs,
		Disk:       disk,
		QemuBin:    qemuBin,
		AllPresent: launcher && kernel && initramfs && disk && qemuBin,
	}
}

func fileExistsAny(paths []string) bool {
	for _, p := range paths {
		if fi, err := os.Stat(p); err == nil && !fi.IsDir() {
			return true
		}
	}
	return false
}

func getNightlyUrl() string {
	base := "https://nightly.link/itouakirai/wrapper/workflows/build-lite/lite"
	switch runtime.GOOS {
	case "windows":
		return base + "/wrapper-lite-qemu-windows-x86_64.zip"
	case "darwin":
		return base + "/wrapper-lite-qemu-macos-aarch64.zip"
	case "linux":
		if runtime.GOARCH == "arm64" {
			return base + "/wrapper-lite-qemu-linux-aarch64.zip"
		}
		return base + "/wrapper-lite-qemu-linux-x86_64.zip"
	case "android":
		return base + "/wrapper-lite-qemu-android-aarch64.zip"
	default:
		return base + "/wrapper-lite-qemu-windows-x86_64.zip"
	}
}

func (s *AppState) downloadQemuPackage() {
	urlStr := getNightlyUrl()
	s.appendLog(fmt.Sprintf("[pkg] Initiating download from %s", urlStr))
	s.broadcastDownload(DownloadProgress{Status: "downloading", Percent: 0})

	transport := &http.Transport{
		Proxy: http.ProxyFromEnvironment,
		DialContext: (&net.Dialer{
			Timeout:   15 * time.Second,
			KeepAlive: 30 * time.Second,
		}).DialContext,
		TLSHandshakeTimeout: 15 * time.Second,
	}

	s.mu.RLock()
	proxyStr := s.config.Proxy
	s.mu.RUnlock()
	if proxyStr != "" {
		if !strings.Contains(proxyStr, "://") {
			proxyStr = "http://" + proxyStr
		}
		if proxyURL, err := url.Parse(proxyStr); err == nil {
			transport.Proxy = http.ProxyURL(proxyURL)
		}
	}

	client := &http.Client{
		Transport: transport,
	}

	resp, err := client.Get(urlStr)
	if err != nil {
		s.appendLog(fmt.Sprintf("[error] Failed to initiate download: %v", err))
		s.broadcastDownload(DownloadProgress{Status: "error", Message: err.Error()})
		return
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		errStr := fmt.Sprintf("Server returned status %d", resp.StatusCode)
		s.appendLog(fmt.Sprintf("[error] Download failed: %s", errStr))
		s.broadcastDownload(DownloadProgress{Status: "error", Message: errStr})
		return
	}

	totalSize := resp.ContentLength
	tmpFile, err := os.CreateTemp("", "wl-qemu-*.zip")
	if err != nil {
		s.broadcastDownload(DownloadProgress{Status: "error", Message: err.Error()})
		return
	}
	defer os.Remove(tmpFile.Name())
	defer tmpFile.Close()

	var downloaded int64
	buf := make([]byte, 64*1024)
	lastTime := time.Now()
	var lastDownloaded int64

	for {
		n, err := resp.Body.Read(buf)
		if n > 0 {
			_, _ = tmpFile.Write(buf[:n])
			downloaded += int64(n)

			now := time.Now()
			elapsed := now.Sub(lastTime).Seconds()
			if elapsed >= 0.5 {
				speed := int64(float64(downloaded-lastDownloaded) / elapsed)
				pct := float64(0)
				if totalSize > 0 {
					pct = float64(downloaded) / float64(totalSize) * 100.0
				}
				s.broadcastDownload(DownloadProgress{
					Status:     "downloading",
					Percent:    pct,
					Downloaded: downloaded,
					Total:      totalSize,
					Speed:      speed,
				})
				lastTime = now
				lastDownloaded = downloaded
			}
		}
		if err != nil {
			if err == io.EOF {
				break
			}
			s.broadcastDownload(DownloadProgress{Status: "error", Message: err.Error()})
			return
		}
	}

	// Extraction
	s.broadcastDownload(DownloadProgress{Status: "extracting", Percent: 99})
	s.appendLog("[pkg] Download completed. Extracting archive...")

	err = extractZip(tmpFile.Name(), s.appDir)
	if err != nil {
		s.appendLog(fmt.Sprintf("[error] Failed to extract archive: %v", err))
		s.broadcastDownload(DownloadProgress{Status: "error", Message: err.Error()})
		return
	}

	s.appendLog("[pkg] Package extracted successfully. Ready to run!")
	s.broadcastDownload(DownloadProgress{Status: "done", Percent: 100})
}

func extractZip(zipPath, destDir string) error {
	r, err := zip.OpenReader(zipPath)
	if err != nil {
		return err
	}
	defer r.Close()

	currentExe, _ := os.Executable()

	for _, f := range r.File {
		fpath := filepath.Join(destDir, f.Name)
		// Guard against Zip Slip vulnerability
		cleanDest := filepath.Clean(destDir)
		cleanPath := filepath.Clean(fpath)
		if !strings.HasPrefix(cleanPath, cleanDest+string(os.PathSeparator)) && cleanPath != cleanDest {
			continue
		}

		// Prevent Windows Access Denied error when attempting to overwrite the currently executing GUI binary
		if currentExe != "" && strings.EqualFold(cleanPath, filepath.Clean(currentExe)) {
			continue
		}

		if f.FileInfo().IsDir() {
			_ = os.MkdirAll(fpath, 0755)
			continue
		}

		if err := os.MkdirAll(filepath.Dir(fpath), 0755); err != nil {
			return err
		}

		outFile, err := os.OpenFile(fpath, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, f.Mode())
		if err != nil {
			return err
		}

		rc, err := f.Open()
		if err != nil {
			outFile.Close()
			return err
		}

		_, err = io.Copy(outFile, rc)
		outFile.Close()
		rc.Close()
		if err != nil {
			return err
		}

		// Ensure executables and shared libraries have proper permissions on Unix
		if runtime.GOOS != "windows" {
			if strings.Contains(f.Name, "bin/") ||
				strings.HasPrefix(f.Name, "wrapper-lite") ||
				strings.HasSuffix(f.Name, ".so") ||
				(f.FileInfo().Mode()&0111 != 0) {
				_ = os.Chmod(fpath, 0755)
			}
		}
	}
	return nil
}

func openBrowser(url string) {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "windows":
		cmd = exec.Command("cmd", "/c", "start", "", url)
	case "darwin":
		cmd = exec.Command("open", url)
	default:
		cmd = exec.Command("xdg-open", url)
	}
	_ = cmd.Start()
}
