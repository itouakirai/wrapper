/**
 * wrapper-lite Cross-Platform GUI Core JavaScript
 * Supports Desktop (Windows, macOS, Linux) and Mobile (Android via WebView Bridge)
 */

// State
let isRunning = false;
let serviceStartTime = null;
let uptimeInterval = null;
let statusPollInterval = null;
let logEventSource = null;
let logsCount = 0;
let platformInfo = {
  os: 'unknown',
  arch: 'unknown',
  isAndroid: false,
  hasQemu: false,
  hasNative: false,
  nightlyUrl: ''
};

// Detect Android Native Bridge
const isAndroidApp = typeof window.Android !== 'undefined';

// Initialization
document.addEventListener('DOMContentLoaded', () => {
  initTheme();
  initTabs();
  detectPlatform();
  loadSavedSettings();
  startStatusPolling();
  initLogStream();
  checkQemuPackageStatus();
});

// Theme Management
function initTheme() {
  const savedTheme = localStorage.getItem('wl_theme') || 'dark';
  setTheme(savedTheme);

  document.getElementById('theme-toggle-btn').addEventListener('click', () => {
    const currentTheme = document.body.classList.contains('light-theme') ? 'light' : 'dark';
    const newTheme = currentTheme === 'dark' ? 'light' : 'dark';
    setTheme(newTheme);
  });
}

function setTheme(theme) {
  const moonIcon = document.getElementById('moon-icon');
  const sunIcon = document.getElementById('sun-icon');
  if (theme === 'light') {
    document.body.classList.remove('dark-theme');
    document.body.classList.add('light-theme');
    moonIcon.classList.add('hidden');
    sunIcon.classList.remove('hidden');
  } else {
    document.body.classList.remove('light-theme');
    document.body.classList.add('dark-theme');
    moonIcon.classList.remove('hidden');
    sunIcon.classList.add('hidden');
  }
  localStorage.setItem('wl_theme', theme);
}

// Navigation Tabs
function initTabs() {
  const tabs = document.querySelectorAll('.tab-btn');
  tabs.forEach(tab => {
    tab.addEventListener('click', () => {
      const tabName = tab.dataset.tab;
      switchToTab(tabName);
    });
  });
}

function switchToTab(tabName) {
  document.querySelectorAll('.tab-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.tab === tabName);
  });
  document.querySelectorAll('.tab-pane').forEach(pane => {
    pane.classList.toggle('active', pane.id === `tab-${tabName}`);
  });
}

// Platform Detection
async function detectPlatform() {
  if (isAndroidApp) {
    platformInfo.isAndroid = true;
    platformInfo.os = 'android';
    platformInfo.arch = 'aarch64';
    updatePlatformUI();
    return;
  }

  try {
    const res = await fetch('/api/info');
    if (res.ok) {
      const data = await res.json();
      platformInfo = { ...platformInfo, ...data };
      updatePlatformUI();
    }
  } catch (err) {
    console.warn('Could not fetch platform info:', err);
  }
}

function updatePlatformUI() {
  const badge = document.getElementById('platform-badge');
  const metricBadge = document.getElementById('metric-platform-badge');
  const osLabel = document.getElementById('detected-os-label');
  const engineSelect = document.getElementById('cfg-engine-mode');

  let osName = 'Desktop';
  if (platformInfo.os === 'windows') osName = 'Windows (x86_64)';
  else if (platformInfo.os === 'darwin' || platformInfo.os === 'macos') osName = 'macOS';
  else if (platformInfo.os === 'linux') osName = 'Linux';
  else if (platformInfo.os === 'android') osName = 'Android';

  if (badge) badge.innerText = `${osName} GUI`;
  if (metricBadge) metricBadge.innerText = osName;
  if (osLabel) osLabel.innerText = osName;

  // On non-Linux, disable native mode
  if (engineSelect && platformInfo.os !== 'linux') {
    engineSelect.value = 'qemu';
    const nativeOpt = engineSelect.querySelector('option[value="native"]');
    if (nativeOpt) nativeOpt.disabled = true;
  }
}

// Settings
function loadSavedSettings() {
  const defaults = {
    host: '127.0.0.1',
    port: 12340,
    proxy: '',
    engine: 'qemu',
    memory: '512',
    smp: '2',
    accel: 'auto',
    logLevel: 'info',
    refreshInterval: '1800'
  };

  let saved = {};
  if (isAndroidApp && window.Android.loadConfig) {
    try {
      saved = JSON.parse(window.Android.loadConfig() || '{}');
    } catch (e) {}
  } else {
    try {
      saved = JSON.parse(localStorage.getItem('wl_config') || '{}');
    } catch (e) {}
  }

  const cfg = { ...defaults, ...saved };
  document.getElementById('cfg-host').value = cfg.host;
  document.getElementById('cfg-host-port').value = cfg.port;
  document.getElementById('cfg-proxy').value = cfg.proxy;
  document.getElementById('cfg-memory').value = cfg.memory;
  document.getElementById('val-memory').innerText = cfg.memory + ' MB';
  document.getElementById('cfg-smp').value = cfg.smp;
  document.getElementById('val-smp').innerText = cfg.smp + ' Cores';
  document.getElementById('cfg-accel').value = cfg.accel;
  document.getElementById('cfg-log-level').value = cfg.logLevel;
  document.getElementById('auth-refresh-interval').value = cfg.refreshInterval;

  updateEndpointDisplay(cfg.host, cfg.port);
}

function saveSettings() {
  const cfg = {
    host: document.getElementById('cfg-host').value.trim() || '127.0.0.1',
    port: parseInt(document.getElementById('cfg-host-port').value, 10) || 12340,
    proxy: document.getElementById('cfg-proxy').value.trim(),
    engine: document.getElementById('cfg-engine-mode').value,
    memory: document.getElementById('cfg-memory').value,
    smp: document.getElementById('cfg-smp').value,
    accel: document.getElementById('cfg-accel').value,
    logLevel: document.getElementById('cfg-log-level').value,
    refreshInterval: document.getElementById('auth-refresh-interval').value
  };

  const json = JSON.stringify(cfg);
  if (isAndroidApp && window.Android.saveConfig) {
    window.Android.saveConfig(json);
  } else {
    localStorage.setItem('wl_config', json);
  }

  updateEndpointDisplay(cfg.host, cfg.port);
  appendLog('[gui] Configuration saved successfully.', 'info');
  alert('Settings saved!');
}

function resetSettings() {
  if (confirm('Reset settings to default?')) {
    localStorage.removeItem('wl_config');
    loadSavedSettings();
  }
}

function setHostPreset(host) {
  document.getElementById('cfg-host').value = host;
  const port = document.getElementById('cfg-host-port').value;
  updateEndpointDisplay(host, port);
}

function updateEndpointDisplay(host, port) {
  const displayUrl = `http://${host}:${port}`;
  const el = document.getElementById('display-service-url');
  if (el) el.innerText = displayUrl;
}

// Service Lifecycle
async function handleStart() {
  setStartingUI();
  appendLog('[gui] Launching wrapper-lite service...', 'system');

  const config = {
    host: document.getElementById('cfg-host').value.trim() || '127.0.0.1',
    port: parseInt(document.getElementById('cfg-host-port').value, 10) || 12340,
    proxy: document.getElementById('cfg-proxy').value.trim(),
    engine: document.getElementById('cfg-engine-mode').value,
    memory: document.getElementById('cfg-memory').value,
    smp: document.getElementById('cfg-smp').value,
    accel: document.getElementById('cfg-accel').value,
    logLevel: document.getElementById('cfg-log-level').value,
    refreshInterval: document.getElementById('auth-refresh-interval').value
  };

  if (isAndroidApp && window.Android.startService) {
    window.Android.startService(JSON.stringify(config));
    return;
  }

  try {
    const res = await fetch('/api/start', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(config)
    });
    const data = await res.json();
    if (!res.ok) {
      appendLog(`[error] Failed to start service: ${data.message || 'Unknown error'}`, 'error');
      setStoppedUI();
      alert(`Start failed: ${data.message}`);
    }
  } catch (err) {
    appendLog(`[error] Network error when starting service: ${err.message}`, 'error');
    setStoppedUI();
  }
}

async function handleStop() {
  appendLog('[gui] Stopping wrapper-lite service...', 'system');

  if (isAndroidApp && window.Android.stopService) {
    window.Android.stopService();
    setStoppedUI();
    return;
  }

  try {
    const res = await fetch('/api/stop', { method: 'POST' });
    if (res.ok) {
      setStoppedUI();
      appendLog('[gui] Service stopped.', 'system');
    }
  } catch (err) {
    appendLog(`[error] Failed to stop service: ${err.message}`, 'error');
  }
}

async function handleRestart() {
  await handleStop();
  setTimeout(handleStart, 1500);
}

function openServiceUrl() {
  const host = document.getElementById('cfg-host').value.trim() || '127.0.0.1';
  const port = document.getElementById('cfg-host-port').value.trim() || '12340';
  const url = `http://${host === '0.0.0.0' ? '127.0.0.1' : host}:${port}/status`;
  
  if (isAndroidApp && window.Android.openBrowser) {
    window.Android.openBrowser(url);
  } else {
    window.open(url, '_blank');
  }
}

function copyServiceUrl() {
  const host = document.getElementById('cfg-host').value.trim() || '127.0.0.1';
  const port = document.getElementById('cfg-host-port').value.trim() || '12340';
  const url = `http://${host}:${port}`;
  navigator.clipboard.writeText(url).then(() => {
    alert(`Copied: ${url}`);
  });
}

// UI State Toggles
function setRunningUI(regions = []) {
  isRunning = true;
  document.getElementById('btn-start').classList.add('hidden');
  document.getElementById('btn-stop').classList.remove('hidden');

  const dot = document.getElementById('status-dot');
  dot.className = 'status-dot running';
  document.getElementById('status-text').innerText = 'Running';

  document.getElementById('metric-state').innerText = 'Running';
  const stateBadge = document.getElementById('metric-state-badge');
  stateBadge.innerText = 'Online';
  stateBadge.className = 'badge badge-info';

  if (!serviceStartTime) serviceStartTime = Date.now();
  startUptimeTracker();

  if (regions && regions.length > 0) {
    document.getElementById('metric-regions').innerText = regions.join(', ');
    document.getElementById('metric-regions-hint').innerText = `${regions.length} storefront(s) active`;
  }
}

function setStoppedUI() {
  isRunning = false;
  serviceStartTime = null;
  stopUptimeTracker();

  document.getElementById('btn-start').classList.remove('hidden');
  document.getElementById('btn-stop').classList.add('hidden');

  const dot = document.getElementById('status-dot');
  dot.className = 'status-dot stopped';
  document.getElementById('status-text').innerText = 'Stopped';

  document.getElementById('metric-state').innerText = 'Stopped';
  const stateBadge = document.getElementById('metric-state-badge');
  stateBadge.innerText = 'Inactive';
  stateBadge.className = 'badge';

  document.getElementById('metric-uptime').innerText = 'Uptime: 0s';
  document.getElementById('metric-regions').innerText = 'None';
  document.getElementById('metric-regions-hint').innerText = 'Available after successful boot';
  document.getElementById('metric-latency').innerText = '-- ms';
  document.getElementById('metric-ping-badge').innerText = '--';
}

function setStartingUI() {
  document.getElementById('status-dot').className = 'status-dot starting';
  document.getElementById('status-text').innerText = 'Starting...';
  document.getElementById('metric-state').innerText = 'Booting...';
}

function startUptimeTracker() {
  if (uptimeInterval) clearInterval(uptimeInterval);
  uptimeInterval = setInterval(() => {
    if (!serviceStartTime) return;
    const diff = Math.floor((Date.now() - serviceStartTime) / 1000);
    const m = Math.floor(diff / 60);
    const s = diff % 60;
    const text = m > 0 ? `${m}m ${s}s` : `${s}s`;
    document.getElementById('metric-uptime').innerText = `Uptime: ${text}`;
  }, 1000);
}

function stopUptimeTracker() {
  if (uptimeInterval) {
    clearInterval(uptimeInterval);
    uptimeInterval = null;
  }
}

// Polling & Health Checks
function startStatusPolling() {
  if (statusPollInterval) clearInterval(statusPollInterval);
  statusPollInterval = setInterval(checkServiceHealth, 3000);
  checkServiceHealth();
}

async function checkServiceHealth() {
  // Check local backend status
  let backendRunning = false;
  if (isAndroidApp && window.Android.isServiceRunning) {
    backendRunning = window.Android.isServiceRunning();
  } else {
    try {
      const res = await fetch('/api/status');
      if (res.ok) {
        const data = await res.json();
        backendRunning = data.running;
      }
    } catch (e) {}
  }

  if (!backendRunning) {
    if (isRunning) setStoppedUI();
    return;
  }

  // Ping the wrapper-lite HTTP port directly or via proxy
  const host = document.getElementById('cfg-host').value.trim() || '127.0.0.1';
  const port = document.getElementById('cfg-host-port').value.trim() || '12340';
  const pingTarget = host === '0.0.0.0' ? '127.0.0.1' : host;
  const startPing = Date.now();

  try {
    const res = await fetch(`http://${pingTarget}:${port}/status`, { signal: AbortSignal.timeout(2000) });
    const latency = Date.now() - startPing;
    if (res.ok) {
      const data = await res.json();
      const regions = data.data && data.data.regions ? data.data.regions : [];
      setRunningUI(regions);
      document.getElementById('metric-latency').innerText = `${latency} ms`;
      const pingBadge = document.getElementById('metric-ping-badge');
      pingBadge.innerText = latency < 100 ? 'Good' : 'Normal';
      pingBadge.className = 'badge badge-info';
    } else {
      setStartingUI();
    }
  } catch (err) {
    // Port not answering yet (guest still booting)
    if (backendRunning) {
      setStartingUI();
    }
  }
}

// Authentication & Login Flow
async function handleLogin() {
  const username = document.getElementById('auth-username').value.trim();
  const password = document.getElementById('auth-password').value;
  const twoFactor = document.getElementById('auth-2fa').value.trim();
  const progressBox = document.getElementById('login-progress-box');
  const progressMsg = document.getElementById('login-progress-msg');
  const submitBtn = document.getElementById('btn-login-submit');

  if (!username || !password) {
    alert('Please enter both Apple ID and password');
    return;
  }

  submitBtn.disabled = true;
  progressBox.classList.remove('hidden');
  progressMsg.innerText = 'Connecting to Apple Music authentication...';
  appendLog(`[auth] Initiating login for account ${username}...`, 'info');

  const payload = {
    username,
    password,
    twoFactor,
    deviceInfo: document.getElementById('auth-device-info').value.trim(),
    proxy: document.getElementById('cfg-proxy').value.trim()
  };

  if (isAndroidApp && window.Android.login) {
    window.Android.login(JSON.stringify(payload));
    return;
  }

  try {
    const res = await fetch('/api/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    const result = await res.json();
    if (res.ok && result.success) {
      appendLog('[auth] Login succeeded! Decryption tokens cached.', 'run');
      alert('Login successful! Tokens are cached.');
    } else if (result.need2FA) {
      prompt2faModal();
    } else {
      appendLog(`[error] Login failed: ${result.message || 'Authentication error'}`, 'error');
      alert(`Login failed: ${result.message || 'Check credentials'}`);
    }
  } catch (err) {
    appendLog(`[error] Login request error: ${err.message}`, 'error');
    alert(`Error: ${err.message}`);
  } finally {
    submitBtn.disabled = false;
    progressBox.classList.add('hidden');
  }
}

function prompt2faModal() {
  document.getElementById('modal-2fa').classList.remove('hidden');
  document.getElementById('modal-2fa-input').focus();
}

function close2faModal() {
  document.getElementById('modal-2fa').classList.add('hidden');
}

async function submit2faCode() {
  const code = document.getElementById('modal-2fa-input').value.trim();
  if (!code || code.length !== 6) {
    alert('Please enter a 6-digit code');
    return;
  }
  close2faModal();
  document.getElementById('auth-2fa').value = code;
  handleLogin();
}

// QEMU Package Manager
async function checkQemuPackageStatus() {
  if (isAndroidApp && window.Android.checkQemuStatus) {
    try {
      const data = JSON.parse(window.Android.checkQemuStatus());
      updateQemuChecklist(data);
    } catch (e) {}
    return;
  }

  try {
    const res = await fetch('/api/qemu/status');
    if (res.ok) {
      const data = await res.json();
      updateQemuChecklist(data);
    }
  } catch (err) {
    console.warn('QEMU status check failed:', err);
  }
}

function updateQemuChecklist(data) {
  const items = [
    { id: 'launcher', ok: data.launcher, name: 'wrapper-lite-qemu' },
    { id: 'kernel', ok: data.kernel, name: 'vmlinuz-lite-qemu' },
    { id: 'initramfs', ok: data.initramfs, name: 'lite-initramfs.cpio.gz' },
    { id: 'disk', ok: data.disk, name: 'data.img' },
    { id: 'qemubin', ok: data.qemuBin, name: 'qemu-system-x86_64' }
  ];

  let allReady = true;
  items.forEach(item => {
    const icon = document.getElementById(`icon-${item.id}`);
    const detail = document.getElementById(`detail-${item.id}`);
    if (icon && detail) {
      if (item.ok) {
        icon.innerText = '✅';
        detail.innerText = 'Present and verified';
        detail.style.color = 'var(--success-color)';
      } else {
        allReady = false;
        icon.innerText = '❌';
        detail.innerText = 'Missing - required for QEMU mode';
        detail.style.color = 'var(--danger-color)';
      }
    }
  });

  const missingAlert = document.getElementById('qemu-missing-alert');
  if (missingAlert) {
    missingAlert.classList.toggle('hidden', allReady);
  }
}

async function startDownloadQemuPackage() {
  const btn = document.getElementById('btn-download-qemu');
  const progressCard = document.getElementById('qemu-download-progress-card');
  const progressBar = document.getElementById('qemu-download-progress-bar');
  const pctText = document.getElementById('qemu-download-pct');
  const statusText = document.getElementById('qemu-download-status-text');
  const bytesText = document.getElementById('qemu-download-bytes');
  const speedText = document.getElementById('qemu-download-speed');

  btn.disabled = true;
  progressCard.classList.remove('hidden');
  progressBar.style.width = '0%';
  pctText.innerText = '0%';
  statusText.innerText = 'Connecting to nightly download server...';

  appendLog('[pkg] Starting automatic download of prebuilt QEMU all-in-one package...', 'info');

  if (isAndroidApp && window.Android.downloadQemuPackage) {
    window.Android.downloadQemuPackage();
    return;
  }

  try {
    const res = await fetch('/api/qemu/download', { method: 'POST' });
    if (!res.ok) {
      const err = await res.json();
      throw new Error(err.message || 'Download trigger failed');
    }

    // Subscribe to SSE progress
    const sse = new EventSource('/api/qemu/download/progress');
    sse.onmessage = (event) => {
      try {
        const d = JSON.parse(event.data);
        if (d.status === 'downloading') {
          const pct = Math.floor(d.percent);
          progressBar.style.width = `${pct}%`;
          pctText.innerText = `${pct}%`;
          statusText.innerText = `Downloading package (${pct}%)...`;
          bytesText.innerText = `${(d.downloaded / 1048576).toFixed(1)} MB / ${(d.total / 1048576).toFixed(1)} MB`;
          speedText.innerText = `${(d.speed / 1024).toFixed(1)} KB/s`;
        } else if (d.status === 'extracting') {
          progressBar.style.width = '99%';
          pctText.innerText = '99%';
          statusText.innerText = 'Extracting and verifying assets...';
        } else if (d.status === 'done') {
          sse.close();
          progressBar.style.width = '100%';
          pctText.innerText = '100%';
          statusText.innerText = 'Installation completed!';
          btn.disabled = false;
          appendLog('[pkg] QEMU all-in-one package installed successfully!', 'run');
          setTimeout(() => {
            progressCard.classList.add('hidden');
            checkQemuPackageStatus();
          }, 2000);
        } else if (d.status === 'error') {
          sse.close();
          btn.disabled = false;
          statusText.innerText = `Error: ${d.message}`;
          appendLog(`[error] QEMU download failed: ${d.message}`, 'error');
        }
      } catch (e) {}
    };

    sse.onerror = () => {
      sse.close();
      btn.disabled = false;
    };
  } catch (err) {
    btn.disabled = false;
    statusText.innerText = `Download failed: ${err.message}`;
    appendLog(`[error] Download failed: ${err.message}`, 'error');
  }
}

// API Tester
function handleTesterEndpointChange() {
  const ep = document.getElementById('test-endpoint-select').value;
  const adamGroup = document.getElementById('test-adamid-group');
  const uriGroup = document.getElementById('test-uri-group');

  if (ep === '/status') {
    adamGroup.classList.add('hidden');
    uriGroup.classList.add('hidden');
  } else if (ep === '/key') {
    adamGroup.classList.remove('hidden');
    uriGroup.classList.remove('hidden');
  } else {
    adamGroup.classList.remove('hidden');
    uriGroup.classList.add('hidden');
  }
}

function openTesterWithEndpoint(ep) {
  switchToTab('tester');
  const select = document.getElementById('test-endpoint-select');
  select.value = ep;
  handleTesterEndpointChange();
}

function quickTest(ep) {
  openTesterWithEndpoint(ep);
  executeApiTest();
}

async function executeApiTest() {
  const ep = document.getElementById('test-endpoint-select').value;
  const adamId = document.getElementById('test-adamid').value.trim();
  const uri = document.getElementById('test-uri').value.trim();
  const host = document.getElementById('cfg-host').value.trim() || '127.0.0.1';
  const port = document.getElementById('cfg-host-port').value.trim() || '12340';
  const targetHost = host === '0.0.0.0' ? '127.0.0.1' : host;

  let query = '';
  if (ep !== '/status' && adamId) {
    query += `?adamId=${encodeURIComponent(adamId)}`;
    if (ep === '/key' && uri) {
      query += `&uri=${encodeURIComponent(uri)}`;
    }
  }

  const url = `http://${targetHost}:${port}${ep}${query}`;
  const statusEl = document.getElementById('test-response-status');
  const bodyEl = document.getElementById('test-response-body');

  statusEl.innerText = 'Requesting...';
  bodyEl.innerText = 'Waiting for response from ' + url;

  try {
    const start = Date.now();
    const res = await fetch(url);
    const latency = Date.now() - start;
    statusEl.innerText = `${res.status} ${res.statusText} (${latency}ms)`;

    const text = await res.text();
    try {
      const obj = JSON.parse(text);
      bodyEl.innerHTML = `<code>${escapeHtml(JSON.stringify(obj, null, 2))}</code>`;
    } catch (e) {
      bodyEl.innerHTML = `<code>${escapeHtml(text)}</code>`;
    }
  } catch (err) {
    statusEl.innerText = 'Failed';
    bodyEl.innerHTML = `<code>Error: ${escapeHtml(err.message)}\nIs wrapper-lite running on ${targetHost}:${port}?</code>`;
  }
}

function copyTestResponse() {
  const text = document.getElementById('test-response-body').innerText;
  navigator.clipboard.writeText(text).then(() => {
    alert('Response copied to clipboard!');
  });
}

// Live Logs Terminal
function initLogStream() {
  if (isAndroidApp) {
    // Android calls window.onAndroidLogEntry(line)
    window.onAndroidLogEntry = (line) => {
      appendLog(line);
    };
    return;
  }

  try {
    logEventSource = new EventSource('/api/logs/stream');
    logEventSource.onmessage = (event) => {
      if (event.data) appendLog(event.data);
    };
    logEventSource.onerror = () => {
      // Reconnect automatically handled by EventSource
    };
  } catch (e) {}
}

function appendLog(line, forcedClass = null) {
  const terminal = document.getElementById('logs-terminal');
  if (!terminal) return;

  const div = document.createElement('div');
  div.className = 'log-line';

  if (forcedClass) {
    div.classList.add(`log-${forcedClass}`);
  } else if (line.includes('[error]') || line.includes('ERROR') || line.includes('failed')) {
    div.classList.add('log-error');
  } else if (line.includes('[warn]') || line.includes('WARN')) {
    div.classList.add('log-warn');
  } else if (line.includes('[run]') || line.includes('SUCCESS')) {
    div.classList.add('log-run');
  } else if (line.includes('[info]') || line.includes('INFO')) {
    div.classList.add('log-info');
  } else {
    div.classList.add('log-system');
  }

  div.innerText = line;
  terminal.appendChild(div);

  logsCount++;
  const badge = document.getElementById('logs-count-badge');
  if (badge) badge.innerText = `${logsCount} lines`;

  // Auto-scroll
  if (document.getElementById('chk-autoscroll').checked) {
    terminal.scrollTop = terminal.scrollHeight;
  }
}

function clearLogs() {
  const terminal = document.getElementById('logs-terminal');
  if (terminal) terminal.innerHTML = '';
  logsCount = 0;
  document.getElementById('logs-count-badge').innerText = '0 lines';
}

function copyAllLogs() {
  const terminal = document.getElementById('logs-terminal');
  if (!terminal) return;
  navigator.clipboard.writeText(terminal.innerText).then(() => {
    alert('Logs copied to clipboard!');
  });
}

function escapeHtml(str) {
  return str.replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;');
}

// Android Bridge Callbacks
window.onAndroidStatusUpdate = (statusJson) => {
  try {
    const data = JSON.parse(statusJson);
    if (data.running) {
      setRunningUI(data.regions);
    } else {
      setStoppedUI();
    }
  } catch (e) {}
};

window.onAndroidDownloadProgress = (pct, speed, status) => {
  const btn = document.getElementById('btn-download-qemu');
  const progressCard = document.getElementById('qemu-download-progress-card');
  const progressBar = document.getElementById('qemu-download-progress-bar');
  const pctText = document.getElementById('qemu-download-pct');
  const statusText = document.getElementById('qemu-download-status-text');
  const speedText = document.getElementById('qemu-download-speed');

  if (progressBar) progressBar.style.width = `${pct}%`;
  if (pctText) pctText.innerText = `${pct}%`;
  if (statusText) statusText.innerText = status || 'Downloading...';
  if (speedText) speedText.innerText = speed || '';

  if (pct >= 100) {
    if (btn) btn.disabled = false;
    appendLog('[pkg] QEMU all-in-one package installed successfully!', 'run');
    setTimeout(() => {
      if (progressCard) progressCard.classList.add('hidden');
      checkQemuPackageStatus();
    }, 2000);
  } else if (status && status.startsWith('Error:')) {
    if (btn) btn.disabled = false;
    appendLog(`[error] QEMU download failed: ${status}`, 'error');
  }
};

window.onAndroidLoginResult = (resultJson) => {
  const progressBox = document.getElementById('login-progress-box');
  const submitBtn = document.getElementById('btn-login-submit');
  if (submitBtn) submitBtn.disabled = false;
  if (progressBox) progressBox.classList.add('hidden');

  let result = {};
  try {
    result = typeof resultJson === 'string' ? JSON.parse(resultJson) : resultJson;
  } catch (e) {
    result = { success: false, message: resultJson };
  }

  if (result.success) {
    appendLog('[auth] Login succeeded! Decryption tokens cached.', 'run');
    alert('Login successful! Tokens are cached.');
  } else if (result.need2FA) {
    prompt2faModal();
  } else {
    appendLog(`[error] Login failed: ${result.message || 'Authentication error'}`, 'error');
    alert(`Login failed: ${result.message || 'Check credentials'}`);
  }
};
