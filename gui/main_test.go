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
