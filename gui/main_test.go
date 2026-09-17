package main

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

func TestHasLoginCacheMarkerFile(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "wl-test-*")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	s := &AppState{
		appDir:  tempDir,
		qemuDir: filepath.Join(tempDir, "qemu"),
	}

	// Case 1: No files present -> false
	if s.hasLoginCache() {
		t.Errorf("expected hasLoginCache to be false when directory is empty")
	}

	// Case 2: .login_cached present -> true
	markerFile := filepath.Join(tempDir, ".login_cached")
	if err := os.WriteFile(markerFile, []byte("2026-09-16T20:00:00Z"), 0644); err != nil {
		t.Fatalf("failed to write marker: %v", err)
	}
	if !s.hasLoginCache() {
		t.Errorf("expected hasLoginCache to be true when .login_cached exists")
	}

	_ = os.Remove(markerFile)

	// Case 3: token_cache.json in rootfs/data -> true
	dataDir := filepath.Join(tempDir, "rootfs", "data")
	if err := os.MkdirAll(dataDir, 0755); err != nil {
		t.Fatalf("failed to create data dir: %v", err)
	}
	tokenFile := filepath.Join(dataDir, "token_cache.json")
	if err := os.WriteFile(tokenFile, []byte(`{"storefront_id":"143441"}`), 0644); err != nil {
		t.Fatalf("failed to write token file: %v", err)
	}
	if !s.hasLoginCache() {
		t.Errorf("expected hasLoginCache to be true when token_cache.json exists")
	}

	// Case 4: data.img exists but contains NO tokens, .login_cached exists -> must return false and clear marker
	_ = os.Remove(tokenFile)
	_ = os.RemoveAll(dataDir)
	emptyDisk := filepath.Join(tempDir, "data.img")
	if err := os.WriteFile(emptyDisk, make([]byte, 1024*1024), 0644); err != nil {
		t.Fatalf("failed to write empty disk: %v", err)
	}
	markerFile2 := filepath.Join(tempDir, ".login_cached")
	if err := os.WriteFile(markerFile2, []byte("2026-09-16T20:00:00Z"), 0644); err != nil {
		t.Fatalf("failed to write marker: %v", err)
	}
	if s.hasLoginCache() {
		t.Errorf("expected hasLoginCache to be false when data.img exists but has no tokens")
	}
	if _, err := os.Stat(markerFile2); err == nil {
		t.Errorf("expected stale marker file to be cleared when disk has no tokens")
	}
}

func TestHasTokensInDiskImage(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "wl-disk-test-*")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	diskPath := filepath.Join(tempDir, "data.img")

	// Case 1: File does not exist
	if hasTokensInDiskImage(diskPath) {
		t.Errorf("expected false for nonexistent disk image")
	}

	// Case 2: File contains only mpl_db directory string (must NOT be treated as login cache)
	contentOnlyMplDb := make([]byte, 1024*1024)
	copy(contentOnlyMplDb[500:], []byte("mpl_db"))
	if err := os.WriteFile(diskPath, contentOnlyMplDb, 0644); err != nil {
		t.Fatalf("failed to write disk file: %v", err)
	}
	if hasTokensInDiskImage(diskPath) {
		t.Errorf("expected false when disk image only contains mpl_db directory name")
	}

	// Case 3: File contains kvs.sqlitedb
	copy(contentOnlyMplDb[1000:], []byte("kvs.sqlitedb"))
	if err := os.WriteFile(diskPath, contentOnlyMplDb, 0644); err != nil {
		t.Fatalf("failed to write disk file: %v", err)
	}
	if !hasTokensInDiskImage(diskPath) {
		t.Errorf("expected true when disk image contains kvs.sqlitedb")
	}

	// Case 4: File contains token across 1MB boundary
	boundaryDisk := make([]byte, 2*1024*1024)
	boundaryIdx := 1024*1024 - 4
	copy(boundaryDisk[boundaryIdx:], []byte("token_cache.json"))
	if err := os.WriteFile(diskPath, boundaryDisk, 0644); err != nil {
		t.Fatalf("failed to write boundary disk file: %v", err)
	}
	if !hasTokensInDiskImage(diskPath) {
		t.Errorf("expected true when token signature crosses 1MB boundary")
	}
}

func TestInfoAndStatusEndpoints(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "wl-api-test-*")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	state.mu.Lock()
	state.appDir = tempDir
	state.qemuDir = filepath.Join(tempDir, "qemu")
	state.isRunning = false
	state.mu.Unlock()

	// /api/info
	req := httptest.NewRequest(http.MethodGet, "/api/info", nil)
	w := httptest.NewRecorder()
	handleInfo(w, req)
	if w.Code != http.StatusOK {
		t.Errorf("expected 200 from /api/info, got %d", w.Code)
	}
	if !bytes.Contains(w.Body.Bytes(), []byte(`"hasLoginCache":false`)) {
		t.Errorf("expected hasLoginCache: false in /api/info response, got: %s", w.Body.String())
	}

	// /api/status
	reqStatus := httptest.NewRequest(http.MethodGet, "/api/status", nil)
	wStatus := httptest.NewRecorder()
	handleStatus(wStatus, reqStatus)
	if wStatus.Code != http.StatusOK {
		t.Errorf("expected 200 from /api/status, got %d", wStatus.Code)
	}
	if !bytes.Contains(wStatus.Body.Bytes(), []byte(`"hasLoginCache":false`)) {
		t.Errorf("expected hasLoginCache: false in /api/status response, got: %s", wStatus.Body.String())
	}
}

func TestEngineModeRestriction(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "wl-engine-test-*")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	s := &AppState{
		appDir:     tempDir,
		qemuDir:    filepath.Join(tempDir, "qemu"),
		logClients: make(map[chan string]bool),
	}

	// On non-Linux or non-x86_64, selecting native must fall back to QEMU
	err = s.startService(Config{
		Engine: "native",
		Host:   "127.0.0.1",
		Port:   12340,
	})
	// wrapper-lite-qemu is not installed in tempDir, so it must return QEMU missing error
	if err == nil {
		t.Errorf("expected error when launcher not found")
	}
	// Verify the error message points to wrapper-lite-qemu (proving it forced QEMU mode)
	expectedErrMsg := "wrapper-lite-qemu not found"
	if !bytes.Contains([]byte(err.Error()), []byte(expectedErrMsg)) {
		t.Errorf("expected error containing %q, got %v", expectedErrMsg, err)
	}
}

func TestExtractErrorMessage(t *testing.T) {
	cases := []struct {
		name     string
		output   string
		expected string
	}{
		{
			name:     "Server message",
			output:   "[run] boot\n[warn] server message: Your Apple ID or password was incorrect.\n[error] login failed\n",
			expected: "Your Apple ID or password was incorrect.",
		},
		{
			name:     "Auth error",
			output:   "[auth] auth error: code=-20101, message=Bad Credentials\n[error] login failed\n",
			expected: "Authentication error: code=-20101, message=Bad Credentials",
		},
		{
			name:     "Auth failed",
			output:   "[auth] auth failed: response type 3\n[error] login failed\n",
			expected: "Authentication failed: response type 3",
		},
		{
			name:     "Generic login failed",
			output:   "[auth] starting\n[error] login failed\n[init] lite exited, powering off...\n",
			expected: "Login failed: incorrect username or password",
		},
		{
			name:     "Token cache failed",
			output:   "[error] failed to cache account info after 5 attempts\n",
			expected: "Login failed: could not cache account tokens",
		},
		{
			name:     "Invalid format",
			output:   "[error] invalid login format, expected user:pass\n",
			expected: "Invalid login format: expected username:password",
		},
		{
			name:     "2FA timeout",
			output:   "[warn] 2FA code timeout (60s), aborting login\n",
			expected: "Two-factor authentication timed out",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := extractErrorMessage(tc.output)
			if got != tc.expected {
				t.Errorf("expected %q, got %q", tc.expected, got)
			}
		})
	}
}

func TestValidateLoginResult(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "wl-validate-login-*")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	qemuDir := filepath.Join(tempDir, "qemu")
	_ = os.MkdirAll(qemuDir, 0755)

	// Case 1: QEMU exit 0 (VM shutdown), but output has "login failed" -> must return success: false
	failedOutput := "[run] starting guest\n[auth] logging in...\n[warn] server message: Bad password\n[error] login failed\n[init] lite exited, powering off...\n"
	res, err := validateLoginResult(failedOutput, nil, true, tempDir, qemuDir)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if res["success"] == true {
		t.Errorf("expected success: false on failed login output")
	}
	if res["message"] != "Bad password" {
		t.Errorf("expected extracted message 'Bad password', got: %v", res["message"])
	}
	// Marker file must NOT exist
	if _, err := os.Stat(filepath.Join(tempDir, ".login_cached")); err == nil {
		t.Errorf("expected .login_cached to not exist after failed login")
	}

	// Case 2: Output requires 2FA
	twoFaOutput := "[auth] credentialHandler: {title: Apple ID, message: Code, 2FA: true}\n2FA code: "
	res2FA, err := validateLoginResult(twoFaOutput, nil, true, tempDir, qemuDir)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if res2FA["success"] == true || res2FA["need2FA"] != true {
		t.Errorf("expected need2FA: true, got: %v", res2FA)
	}

	// Case 3: Output has "login successful", but data.img has no tokens
	emptyDisk := filepath.Join(qemuDir, "data.img")
	_ = os.WriteFile(emptyDisk, make([]byte, 1024*1024), 0644)
	successOutput := "[auth] login successful\n[auth] account info cached successfully\n[auth] login complete, exiting\n"
	resNoTokens, err := validateLoginResult(successOutput, nil, true, tempDir, qemuDir)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if resNoTokens["success"] == true {
		t.Errorf("expected success: false when tokens are not in disk image")
	}

	// Case 4: Output has "login successful" and data.img has tokens -> success: true, marker written
	diskWithTokens := make([]byte, 1024*1024)
	copy(diskWithTokens[500:], []byte("token_cache.json"))
	_ = os.WriteFile(emptyDisk, diskWithTokens, 0644)
	resSuccess, err := validateLoginResult(successOutput, nil, true, tempDir, qemuDir)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if resSuccess["success"] != true {
		t.Errorf("expected success: true, got: %v", resSuccess)
	}
	if _, err := os.Stat(filepath.Join(tempDir, ".login_cached")); err != nil {
		t.Errorf("expected .login_cached to exist after successful login")
	}
}
