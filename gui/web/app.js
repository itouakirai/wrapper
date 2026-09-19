/**
 * wrapper-lite Cross-Platform GUI Core JavaScript
 * Supports Desktop (Windows, macOS, Linux) and Mobile (Android via WebView Bridge)
 */

// State
let isRunning = false;
let isStarting = false;
let serviceStartTime = null;
let uptimeInterval = null;
let statusPollInterval = null;
let logEventSource = null;
let logsCount = 0;
let currentLang = 'en';
let currentRegions = [];
let lastQemuData = null;
let hasCachedLogin = null;
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

// Localization Dictionary
const translations = {
  en: {
    doc_title: "wrapper-lite GUI",
    platform_badge_default: "Cross-Platform GUI",
    platform_gui: "%s GUI",
    status_stopped: "Stopped",
    status_running: "Running",
    status_starting: "Starting...",
    status_booting: "Booting...",
    btn_lang_toggle: "切换为中文 / Switch to Chinese",
    btn_lang_label: "中文",
    btn_theme_toggle: "Toggle Theme",
    btn_start: "Start Service",
    btn_stop: "Stop Service",
    btn_restart: "Restart",
    btn_open_browser: "Project Home",
    btn_project_home: "Project Home",
    lbl_endpoint: "Endpoint:",
    btn_copy_url: "Copy URL",
    tab_dashboard: "Dashboard",
    tab_settings: "Settings",
    tab_account: "Account & Auth",
    tab_qemu: "QEMU Package",
    tab_tester: "API Tester",
    tab_logs: "Live Logs",
    metric_service_state: "Service State",
    badge_inactive: "Inactive",
    badge_online: "Online",
    metric_storefront_regions: "Storefront Regions",
    val_none: "None",
    hint_regions_boot: "Available after successful boot",
    hint_regions_active: "storefront(s) active",
    metric_active_platform: "Active Platform",
    badge_detecting: "Detecting...",
    val_engine_qemu: "QEMU Guest",
    val_engine_native: "Native Rootless",
    hint_engine_vm: "Self-contained VM",
    hint_engine_host: "Host Process",
    metric_api_latency: "API Latency",
    badge_good: "Good",
    badge_normal: "Normal",
    hint_health_check: "Health check /status",
    alert_qemu_missing_title: "Prebuilt QEMU Package Not Installed",
    alert_qemu_missing_desc: "Non-Linux platforms require the precompiled QEMU all-in-one package to run. You can download and install it automatically with one click.",
    btn_download_pkg: "Download Package",
    title_endpoints_ref: "Quick Endpoints Reference",
    th_method: "Method",
    th_endpoint: "Endpoint",
    th_description: "Description",
    th_action: "Action",
    ep_status_desc: "Health check and supported storefront regions",
    ep_m3u8_desc: "Fetch song M3U8 playback stream",
    ep_key_desc: "Fetch track decryption key",
    ep_lyrics_desc: "Fetch lyrics (syllable / line-timed)",
    ep_webplayback_desc: "Fetch web playback tokens",
    btn_test: "Test",
    btn_try: "Try",
    title_network_binding: "Network & Binding",
    lbl_host_address: "Host Listen Address",
    ph_host_address: "127.0.0.1 or 0.0.0.0",
    btn_preset_local: "Local (127.0.0.1)",
    btn_preset_lan: "LAN (0.0.0.0)",
    hint_host_address: "Set to 0.0.0.0 to expose the service to other devices on your local network.",
    lbl_host_port: "Host Listen Port",
    hint_host_port: "The port you connect to on this device (default: 12340).",
    lbl_proxy: "HTTP / SOCKS5 Proxy (Optional)",
    ph_proxy: "e.g. http://127.0.0.1:7890 or socks5://127.0.0.1:1080",
    hint_proxy: "Forward requests through a proxy server.",
    title_qemu_perf: "QEMU & Performance",
    lbl_runtime_mode: "Runtime Mode",
    opt_engine_qemu: "QEMU Virtual Guest (Recommended for Windows/macOS/Android)",
    opt_engine_native: "Native Rootless (Linux x86_64 Only)",
    hint_engine_mode: "Non-Linux x86_64 platforms require QEMU mode.",
    lbl_guest_ram: "Guest RAM (MB)",
    hint_guest_ram: "Allocated RAM for QEMU guest (512MB is optimal).",
    lbl_guest_cpu: "Guest CPU Cores (SMP)",
    lbl_accel: "Hardware Acceleration",
    opt_accel_auto: "Auto (Recommended)",
    opt_accel_whpx: "WHPX (Windows Hypervisor Platform)",
    opt_accel_kvm: "KVM (Linux Kernel Virtual Machine)",
    opt_accel_hvf: "HVF (macOS Hypervisor)",
    opt_accel_tcg: "TCG (Software Emulation / Android Fallback)",
    hint_accel: "Falls back automatically to TCG software emulation if acceleration fails.",
    lbl_log_level: "Log Level",
    btn_save_cfg: "Save Configuration",
    btn_reset_cfg: "Reset to Defaults",
    title_auth: "Apple Music Account Authentication",
    desc_auth: "Logging in caches decryption keys and tokens to data.img (or native data dir). Once tokens are cached, wrapper-lite can run in service mode without re-entering passwords.",
    lbl_apple_id: "Apple ID (Email)",
    ph_apple_id: "name@example.com",
    lbl_password: "Password",
    btn_toggle_password: "Show/Hide Password",
    lbl_2fa: "2FA Verification Code (Optional)",
    ph_2fa: "6-digit code (if prompted on your device)",
    hint_2fa: "If your Apple ID has 2-Step Verification enabled, you can enter the 6-digit code directly here or input it when prompted.",
    lbl_refresh_interval: "Token Auto-Refresh Interval (Seconds)",
    hint_refresh_interval: "How often background token refreshing runs (default: 1800s / 30m).",
    lbl_device_info: "Custom Device Info Override (Optional)",
    ph_device_info: "Leave blank to use auto-generated device identity",
    btn_login: "Login & Cache Tokens",
    msg_logging_in: "Logging in to Apple Music services...",
    title_qemu_pkg: "Precompiled QEMU All-in-One Package",
    desc_qemu_pkg: "The QEMU all-in-one package includes QEMU system binaries, kernel (vmlinuz-lite-qemu), rootfs/initramfs (lite-initramfs.cpio.gz), and preconfigured data disk (data.img).",
    btn_check_status: "Check Status",
    chk_launcher: "Launcher (wrapper-lite-qemu)",
    chk_kernel: "Kernel (vmlinuz-lite-qemu)",
    chk_initramfs: "Initramfs (lite-initramfs.cpio.gz)",
    chk_disk: "Data Disk (data.img)",
    chk_qemubin: "QEMU Binary & Firmware (qemu/bin)",
    status_checking: "Checking...",
    status_present_verified: "Present and verified",
    status_missing_required: "Missing - required for QEMU mode",
    badge_qemu_ready: "All Components Ready",
    badge_qemu_incomplete: "Incomplete / Missing Files",
    badge_checking: "Checking...",
    title_ci_download: "Direct Download from CI Nightly Builds",
    desc_ci_download_pre: "Download the official prebuilt release archive for ",
    desc_ci_download_post: " directly from the nightly builds repository.",
    lbl_source: "Source:",
    btn_download_update_qemu: "Download / Update QEMU Package",
    status_downloading_pkg: "Downloading package...",
    status_extracting_pkg: "Extracting and verifying assets...",
    status_installed_pkg: "Installation completed!",
    status_connecting_download: "Connecting to nightly download server...",
    title_api_tester: "API Test Bench",
    desc_api_tester: "Quickly test Apple Music decryption and metadata endpoints against the running wrapper instance.",
    lbl_tester_ep: "Endpoint",
    opt_ep_status: "GET /status (Health & Storefronts)",
    opt_ep_m3u8: "GET /m3u8 (HLS Playback Stream)",
    opt_ep_lyrics: "GET /lyrics (Time-synced Lyrics)",
    opt_ep_key: "GET /key (Track Key)",
    opt_ep_webplayback: "GET /webplayback (Web Playback)",
    lbl_adam_id: "Adam ID (Song / Track ID)",
    lbl_key_uri: "Key URI",
    btn_send_request: "Send Request",
    lbl_response: "Response:",
    btn_copy_response: "Copy Response",
    ph_response_waiting: "Click \"Send Request\" to test endpoint...",
    status_requesting: "Requesting...",
    msg_waiting_response: "Waiting for response from %s",
    status_failed: "Failed",
    title_live_logs: "Live Console Output",
    lbl_autoscroll: "Auto-scroll",
    btn_clear_logs: "Clear",
    btn_copy_logs: "Copy All",
    log_initialized: "[gui] wrapper-lite GUI initialized.",
    title_2fa_modal: "Two-Factor Authentication",
    desc_2fa_modal: "Apple Music requires a 6-digit 2FA verification code sent to your Apple trusted devices.",
    lbl_enter_code: "Enter 6-digit Code:",
    btn_cancel: "Cancel",
    btn_submit_code: "Submit Code",
    val_cores: "Cores",
    lbl_uptime: "Uptime",
    alert_settings_saved: "Settings saved!",
    log_cfg_saved: "[gui] Configuration saved successfully.",
    confirm_reset_settings: "Reset settings to default?",
    alert_copied: "Copied: %s",
    alert_no_login_cache: "No Apple Music login cache detected! wrapper-lite requires cached login credentials to decrypt audio streams. Please log in first.",
    confirm_start_without_login: "No Apple Music login cache detected. wrapper-lite requires cached login credentials to decrypt audio streams. Do you want to start the service anyway?",
    hint_regions_no_login: "No login cache - login via Account & Auth",
    log_service_starting: "[gui] Launching wrapper-lite service...",
    log_service_start_failed: "[error] Failed to start service: %s",
    alert_start_failed: "Start failed: %s",
    log_service_stopping: "[gui] Stopping wrapper-lite service...",
    log_service_stopped: "[gui] Service stopped.",
    log_service_stop_failed: "[error] Failed to stop service: %s",
    alert_enter_credentials: "Please enter both Apple ID and password",
    log_login_initiating: "[auth] Initiating login for account %s...",
    log_login_succeeded: "[auth] Login succeeded! Decryption tokens cached.",
    alert_login_succeeded: "Login successful! Tokens are cached.",
    log_login_failed: "[error] Login failed: %s",
    alert_login_failed: "Login failed: %s",
    alert_enter_2fa: "Please enter a 6-digit code",
    log_pkg_downloading: "[pkg] Starting automatic download of prebuilt QEMU all-in-one package...",
    log_pkg_installed: "[pkg] QEMU all-in-one package installed successfully!",
    log_pkg_failed: "[error] QEMU download failed: %s",
    alert_response_copied: "Response copied to clipboard!",
    alert_logs_copied: "Logs copied to clipboard!",
    unit_lines: "lines"
  },
  zh: {
    doc_title: "wrapper-lite 图形界面",
    platform_badge_default: "跨平台图形界面",
    platform_gui: "%s 图形界面",
    status_stopped: "已停止",
    status_running: "运行中",
    status_starting: "正在启动...",
    status_booting: "启动中...",
    btn_lang_toggle: "Switch to English / 切换为英文",
    btn_lang_label: "EN",
    btn_theme_toggle: "切换主题",
    btn_start: "启动服务",
    btn_stop: "停止服务",
    btn_restart: "重启服务",
    btn_open_browser: "项目主页",
    btn_project_home: "项目主页",
    lbl_endpoint: "服务接口:",
    btn_copy_url: "复制地址",
    tab_dashboard: "仪表盘",
    tab_settings: "设置",
    tab_account: "账号与认证",
    tab_qemu: "QEMU 软件包",
    tab_tester: "接口测试",
    tab_logs: "实时日志",
    metric_service_state: "服务状态",
    badge_inactive: "未激活",
    badge_online: "在线",
    metric_storefront_regions: "商店地区",
    val_none: "无",
    hint_regions_boot: "服务启动成功后可用",
    hint_regions_active: "个活跃地区",
    metric_active_platform: "运行平台",
    badge_detecting: "检测中...",
    val_engine_qemu: "QEMU 虚拟机",
    val_engine_native: "原生 Rootless 模式",
    hint_engine_vm: "独立隔离虚拟机",
    hint_engine_host: "宿主原生进程",
    metric_api_latency: "接口延迟",
    badge_good: "良好",
    badge_normal: "正常",
    hint_health_check: "健康检查 /status",
    alert_qemu_missing_title: "未安装预编译 QEMU 软件包",
    alert_qemu_missing_desc: "非 Linux 平台需要预编译的 QEMU 一体包支持。您可以一键自动下载并安装。",
    btn_download_pkg: "下载软件包",
    title_endpoints_ref: "常用接口速查",
    th_method: "请求方法",
    th_endpoint: "接口路径",
    th_description: "功能说明",
    th_action: "操作",
    ep_status_desc: "健康检查与支持的商店地区",
    ep_m3u8_desc: "获取歌曲 M3U8 音频播放流",
    ep_key_desc: "获取音频轨道解密密钥",
    ep_lyrics_desc: "获取逐字 / 逐行时间轴歌词",
    ep_webplayback_desc: "获取网页端播放凭据 (Web Playback)",
    btn_test: "测试",
    btn_try: "尝试",
    title_network_binding: "网络与端口绑定",
    lbl_host_address: "监听主机地址",
    ph_host_address: "127.0.0.1 或 0.0.0.0",
    btn_preset_local: "本机 (127.0.0.1)",
    btn_preset_lan: "局域网 (0.0.0.0)",
    hint_host_address: "设置为 0.0.0.0 允许局域网内的其它设备访问服务。",
    lbl_host_port: "服务监听端口",
    hint_host_port: "本设备连接的端口（默认：12340）。",
    lbl_proxy: "HTTP / SOCKS5 代理（可选）",
    ph_proxy: "例如 http://127.0.0.1:7890 或 socks5://127.0.0.1:1080",
    hint_proxy: "通过代理服务器转发网络请求。",
    title_qemu_perf: "QEMU 与性能设置",
    lbl_runtime_mode: "运行模式",
    opt_engine_qemu: "QEMU 虚拟客机（推荐 Windows/macOS/Android）",
    opt_engine_native: "原生 Rootless 模式（仅限 Linux x86_64）",
    hint_engine_mode: "非 Linux x86_64 平台必须使用 QEMU 模式。",
    lbl_guest_ram: "虚拟机分配内存 (MB)",
    hint_guest_ram: "分配给 QEMU 虚拟机的运行内存（512MB 为推荐值）。",
    lbl_guest_cpu: "虚拟机 CPU 核心数 (SMP)",
    lbl_accel: "硬件加速虚拟化",
    opt_accel_auto: "自动检测（推荐）",
    opt_accel_whpx: "WHPX (Windows 虚拟机监控平台)",
    opt_accel_kvm: "KVM (Linux 内核虚拟化)",
    opt_accel_hvf: "HVF (macOS 硬件虚拟化)",
    opt_accel_tcg: "TCG (纯软件模拟 / Android 兼容)",
    hint_accel: "若硬件加速不可用，将自动降级为 TCG 纯软件模拟。",
    lbl_log_level: "日志记录级别",
    btn_save_cfg: "保存配置",
    btn_reset_cfg: "恢复默认设置",
    title_auth: "Apple Music 账号身份认证",
    desc_auth: "登录成功后会将解密密钥与令牌安全缓存至 data.img（或原生数据目录）。完成缓存后，wrapper-lite 即可在服务模式下免密持续运行。",
    lbl_apple_id: "Apple ID（邮箱）",
    ph_apple_id: "name@example.com",
    lbl_password: "密码",
    btn_toggle_password: "显示/隐藏密码",
    lbl_2fa: "双重认证验证码（可选）",
    ph_2fa: "6位数字验证码（若受信任设备已提示）",
    hint_2fa: "若您的 Apple ID 已开启双重认证，可直接在此填写 6 位验证码，或在系统提示时再输入。",
    lbl_refresh_interval: "令牌自动刷新周期（秒）",
    hint_refresh_interval: "后台刷新访问令牌的时间间隔（默认：1800 秒 / 30 分钟）。",
    lbl_device_info: "自定义设备信息覆盖（可选）",
    ph_device_info: "留空将自动生成设备指纹",
    btn_login: "登录并缓存令牌",
    msg_logging_in: "正在向 Apple Music 服务发起登录认证...",
    title_qemu_pkg: "预编译 QEMU 一体化软件包",
    desc_qemu_pkg: "QEMU 一体化软件包包含 QEMU 系统执行程序、系统内核 (vmlinuz-lite-qemu)、内存根文件系统 (lite-initramfs.cpio.gz) 以及预置的数据盘 (data.img)。",
    btn_check_status: "检查就绪状态",
    chk_launcher: "启动引导器 (wrapper-lite-qemu)",
    chk_kernel: "内核镜像 (vmlinuz-lite-qemu)",
    chk_initramfs: "内存盘镜像 (lite-initramfs.cpio.gz)",
    chk_disk: "数据磁盘 (data.img)",
    chk_qemubin: "QEMU 核心程序与固件 (qemu/bin)",
    status_checking: "正在检测...",
    status_present_verified: "已就绪并通过校验",
    status_missing_required: "缺失 - QEMU 模式必须具备",
    badge_qemu_ready: "全部组件就绪",
    badge_qemu_incomplete: "组件不完整 / 缺失",
    badge_checking: "正在检测...",
    title_ci_download: "从 CI Nightly 自动下载",
    desc_ci_download_pre: "直接从官方 Nightly 自动构建仓库下载并安装适配 ",
    desc_ci_download_post: " 的发布包。",
    lbl_source: "来源：",
    btn_download_update_qemu: "下载 / 更新 QEMU 软件包",
    status_downloading_pkg: "正在下载软件包...",
    status_extracting_pkg: "正在解压并校验组件文件...",
    status_installed_pkg: "安装部署完成！",
    status_connecting_download: "正在连接 Nightly 下载服务器...",
    title_api_tester: "API 接口测试台",
    desc_api_tester: "便捷地向正在运行中的 wrapper 服务测试 Apple Music 解密及元数据接口。",
    lbl_tester_ep: "选择测试接口",
    opt_ep_status: "GET /status (健康检查与商店地区)",
    opt_ep_m3u8: "GET /m3u8 (HLS 播放流)",
    opt_ep_lyrics: "GET /lyrics (逐字/逐行歌词)",
    opt_ep_key: "GET /key (音轨解密密钥)",
    opt_ep_webplayback: "GET /webplayback (网页播放凭据)",
    lbl_adam_id: "Adam ID（歌曲 / 音轨 ID）",
    lbl_key_uri: "密钥 URI",
    btn_send_request: "发送测试请求",
    lbl_response: "响应状态：",
    btn_copy_response: "复制响应内容",
    ph_response_waiting: "点击“发送测试请求”查看接口响应...",
    status_requesting: "正在请求...",
    msg_waiting_response: "正在等待 %s 的响应...",
    status_failed: "请求失败",
    title_live_logs: "实时控制台输出",
    lbl_autoscroll: "自动滚屏",
    btn_clear_logs: "清空日志",
    btn_copy_logs: "复制全部",
    log_initialized: "[gui] wrapper-lite 图形界面已初始化。",
    title_2fa_modal: "双重身份认证",
    desc_2fa_modal: "Apple Music 需要验证发送至您受信任 Apple 设备的 6 位双重认证验证码。",
    lbl_enter_code: "请输入 6 位验证码：",
    btn_cancel: "取消",
    btn_submit_code: "提交验证码",
    val_cores: "核",
    lbl_uptime: "运行时间",
    alert_settings_saved: "设置已保存！",
    log_cfg_saved: "[gui] 配置已成功保存。",
    confirm_reset_settings: "是否确认将所有设置恢复为默认值？",
    alert_copied: "已复制到剪贴板: %s",
    alert_no_login_cache: "未检测到 Apple Music 登录缓存！wrapper-lite 依赖有效登录凭据进行音频解密，请先前往「账号与认证」登录！",
    confirm_start_without_login: "检测到尚未登录 Apple Music 账号（无登录缓存）。wrapper-lite 依赖缓存的凭据进行音频解密。是否仍要直接启动服务？",
    hint_regions_no_login: "未检测到有效登录缓存 - 请前往账号与认证登录",
    log_service_starting: "[gui] 正在启动 wrapper-lite 服务...",
    log_service_start_failed: "[error] 启动服务失败: %s",
    alert_start_failed: "启动失败: %s",
    log_service_stopping: "[gui] 正在停止 wrapper-lite 服务...",
    log_service_stopped: "[gui] 服务已停止。",
    log_service_stop_failed: "[error] 停止服务失败: %s",
    alert_enter_credentials: "请输入 Apple ID 和密码",
    log_login_initiating: "[auth] 正在为账号 %s 发起登录认证...",
    log_login_succeeded: "[auth] 登录成功！解密令牌已安全缓存。",
    alert_login_succeeded: "登录成功！令牌已缓存。",
    log_login_failed: "[error] 登录认证失败: %s",
    alert_login_failed: "登录失败: %s",
    alert_enter_2fa: "请输入 6 位双重认证码",
    log_pkg_downloading: "[pkg] 正在开始自动下载预编译 QEMU 一体化软件包...",
    log_pkg_installed: "[pkg] QEMU 一体化软件包安装部署成功！",
    log_pkg_failed: "[error] QEMU 软件包下载失败: %s",
    alert_response_copied: "响应内容已复制到剪贴板！",
    alert_logs_copied: "日志内容已复制到剪贴板！",
    unit_lines: "行"
  }
};

function t(key, ...args) {
  const dict = translations[currentLang] || translations.en;
  let val = dict[key] !== undefined ? dict[key] : (translations.en[key] !== undefined ? translations.en[key] : key);
  if (args.length > 0 && typeof val === 'string') {
    args.forEach(arg => {
      val = val.replace('%s', arg);
    });
  }
  return val;
}

function initLanguage() {
  let lang = 'en';
  try {
    const savedLang = localStorage.getItem('wl_lang');
    if (savedLang === 'zh' || savedLang === 'en') {
      lang = savedLang;
    } else if (navigator.language && navigator.language.toLowerCase().startsWith('zh')) {
      lang = 'zh';
    }
  } catch (e) {}
  setLanguage(lang);
}

function toggleLanguage() {
  setLanguage(currentLang === 'zh' ? 'en' : 'zh');
}

function setLanguage(lang) {
  currentLang = (lang === 'zh') ? 'zh' : 'en';
  try {
    localStorage.setItem('wl_lang', currentLang);
  } catch (e) {}

  document.documentElement.lang = currentLang === 'zh' ? 'zh-CN' : 'en';
  document.title = t('doc_title');

  const langLabel = document.getElementById('lang-text-label');
  if (langLabel) {
    langLabel.innerText = t('btn_lang_label');
  }
  const langBtn = document.getElementById('lang-toggle-btn');
  if (langBtn) {
    langBtn.title = t('btn_lang_toggle');
  }

  // Update all data-i18n elements
  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = el.getAttribute('data-i18n');
    const val = t(key);
    if (val !== undefined && val !== key) {
      el.innerText = val;
    }
  });

  // Update all data-i18n-placeholder elements
  document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
    const key = el.getAttribute('data-i18n-placeholder');
    const val = t(key);
    if (val !== undefined && val !== key) {
      el.placeholder = val;
    }
  });

  // Update all data-i18n-title elements
  document.querySelectorAll('[data-i18n-title]').forEach(el => {
    const key = el.getAttribute('data-i18n-title');
    const val = t(key);
    if (val !== undefined && val !== key) {
      el.title = val;
    }
  });

  // Update dynamic slider values
  const mem = document.getElementById('cfg-memory');
  if (mem) updateMemorySliderDisplay(mem.value);
  const smp = document.getElementById('cfg-smp');
  if (smp) updateSmpSliderDisplay(smp.value);

  // Update platform-dependent and runtime strings
  updatePlatformUI();

  if (isRunning) {
    setRunningUI(currentRegions);
  } else if (isStarting) {
    setStartingUI();
  } else {
    setStoppedUI();
  }

  if (lastQemuData) {
    updateQemuChecklist(lastQemuData);
  }

  const logsBadge = document.getElementById('logs-count-badge');
  if (logsBadge) {
    logsBadge.innerText = `${logsCount} ${t('unit_lines')}`;
  }
  const dashLogsBadge = document.getElementById('dashboard-logs-count-badge');
  if (dashLogsBadge) {
    dashLogsBadge.innerText = `${logsCount} ${t('unit_lines')}`;
  }
}

function updateMemorySliderDisplay(val) {
  const el = document.getElementById('val-memory');
  if (el) el.innerText = `${val} MB`;
}

function updateSmpSliderDisplay(val) {
  const el = document.getElementById('val-smp');
  if (el) el.innerText = `${val} ${t('val_cores')}`;
}

// Initialization
document.addEventListener('DOMContentLoaded', () => {
  initTheme();
  initLanguage();
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
  if (tabName === 'qemu') {
    checkQemuPackageStatus();
  }
}

function isLinuxX86_64Platform() {
  if (platformInfo.isAndroid) return false;
  const os = (platformInfo.os || '').toLowerCase();
  const arch = (platformInfo.arch || '').toLowerCase();
  return os === 'linux' && (arch === 'amd64' || arch === 'x86_64');
}

// Platform Detection
async function detectPlatform() {
  if (isAndroidApp || typeof window.Android !== 'undefined') {
    platformInfo.isAndroid = true;
    platformInfo.os = 'android';
    platformInfo.arch = 'aarch64';
    if (window.Android && typeof window.Android.hasLoginCache === 'function') {
      try {
        hasCachedLogin = window.Android.hasLoginCache();
      } catch (e) {}
    }
    updatePlatformUI();
    if (lastQemuData) {
      updateQemuChecklist(lastQemuData);
    }
    return;
  }

  try {
    const res = await fetch('/api/info');
    if (res.ok) {
      const data = await res.json();
      platformInfo = { ...platformInfo, ...data };
      if (typeof data.hasLoginCache === 'boolean') {
        hasCachedLogin = data.hasLoginCache;
      }
      updatePlatformUI();
    }
  } catch (err) {
    console.warn('Could not fetch platform info:', err);
  }
}

const refreshPlatformInfo = detectPlatform;

function updatePlatformUI() {
  const badge = document.getElementById('platform-badge');
  const metricBadge = document.getElementById('metric-platform-badge');
  const osLabel = document.getElementById('detected-os-label');
  const engineSelect = document.getElementById('cfg-engine-mode');
  const engineMetric = document.getElementById('metric-engine');
  const engineDetail = document.getElementById('metric-engine-detail');

  let osName = 'Desktop';
  if (platformInfo.os === 'windows') osName = 'Windows (x86_64)';
  else if (platformInfo.os === 'darwin' || platformInfo.os === 'macos') osName = 'macOS';
  else if (platformInfo.os === 'linux') {
    const isX86 = platformInfo.arch === 'amd64' || platformInfo.arch === 'x86_64';
    osName = isX86 ? 'Linux (x86_64)' : `Linux (${platformInfo.arch || 'unknown'})`;
  }
  else if (platformInfo.os === 'android') osName = 'Android';

  if (badge) badge.innerText = t('platform_gui', osName);
  if (metricBadge) metricBadge.innerText = osName;
  if (osLabel) osLabel.innerText = osName;

  // On non-Linux x86_64, disable native mode and force QEMU
  const isLinuxX86 = isLinuxX86_64Platform();
  if (engineSelect) {
    const nativeOpt = engineSelect.querySelector('option[value="native"]');
    if (nativeOpt) {
      nativeOpt.disabled = !isLinuxX86;
    }
    if (!isLinuxX86 && engineSelect.value === 'native') {
      engineSelect.value = 'qemu';
    }
  }

  const currentEngine = engineSelect ? engineSelect.value : 'qemu';
  if (engineMetric) {
    engineMetric.innerText = currentEngine === 'native' ? t('val_engine_native') : t('val_engine_qemu');
  }
  if (engineDetail) {
    engineDetail.innerText = currentEngine === 'native' ? t('hint_engine_host') : t('hint_engine_vm');
  }

  const isAndroid = platformInfo.isAndroid || isAndroidApp || (typeof window.Android !== 'undefined');
  const launcherItem = document.getElementById('chk-launcher');
  if (launcherItem && isAndroid) {
    launcherItem.classList.add('hidden');
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
  if (!isLinuxX86_64Platform() && cfg.engine === 'native') {
    cfg.engine = 'qemu';
  }
  document.getElementById('cfg-host').value = cfg.host;
  document.getElementById('cfg-host-port').value = cfg.port;
  document.getElementById('cfg-proxy').value = cfg.proxy;
  if (document.getElementById('cfg-engine-mode')) {
    document.getElementById('cfg-engine-mode').value = cfg.engine || 'qemu';
  }
  document.getElementById('cfg-memory').value = cfg.memory;
  updateMemorySliderDisplay(cfg.memory);
  document.getElementById('cfg-smp').value = cfg.smp;
  updateSmpSliderDisplay(cfg.smp);
  document.getElementById('cfg-accel').value = cfg.accel;
  document.getElementById('cfg-log-level').value = cfg.logLevel;
  document.getElementById('auth-refresh-interval').value = cfg.refreshInterval;

  updateEndpointDisplay(cfg.host, cfg.port);
}

function saveSettings() {
  let engineVal = document.getElementById('cfg-engine-mode').value;
  if (!isLinuxX86_64Platform() && engineVal === 'native') {
    engineVal = 'qemu';
    document.getElementById('cfg-engine-mode').value = 'qemu';
  }

  const cfg = {
    host: document.getElementById('cfg-host').value.trim() || '127.0.0.1',
    port: parseInt(document.getElementById('cfg-host-port').value, 10) || 12340,
    proxy: document.getElementById('cfg-proxy').value.trim(),
    engine: engineVal,
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
  updatePlatformUI();
  appendLog(t('log_cfg_saved'), 'info');
  alert(t('alert_settings_saved'));
}

function resetSettings() {
  if (confirm(t('confirm_reset_settings'))) {
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
  if (isAndroidApp && window.Android && typeof window.Android.hasLoginCache === 'function') {
    try {
      hasCachedLogin = window.Android.hasLoginCache();
    } catch (e) {}
  } else if (!isAndroidApp) {
    try {
      const res = await fetch('/api/status');
      if (res.ok) {
        const data = await res.json();
        if (typeof data.hasLoginCache === 'boolean') {
          hasCachedLogin = data.hasLoginCache;
        }
      }
    } catch (e) {}
  }

  if (!hasCachedLogin) {
    alert(t('alert_no_login_cache'));
    switchToTab('account');
    const userEl = document.getElementById('auth-username');
    if (userEl) userEl.focus();
    return;
  }

  setStartingUI();
  appendLog(t('log_service_starting'), 'system');

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
      appendLog(t('log_service_start_failed', data.message || 'Unknown error'), 'error');
      setStoppedUI();
      alert(t('alert_start_failed', data.message || 'Unknown error'));
    }
  } catch (err) {
    appendLog(t('log_service_start_failed', err.message), 'error');
    setStoppedUI();
  }
}

async function handleStop() {
  appendLog(t('log_service_stopping'), 'system');

  if (isAndroidApp && window.Android.stopService) {
    window.Android.stopService();
    setStoppedUI();
    return;
  }

  try {
    const res = await fetch('/api/stop', { method: 'POST' });
    if (res.ok) {
      setStoppedUI();
      appendLog(t('log_service_stopped'), 'system');
    }
  } catch (err) {
    appendLog(t('log_service_stop_failed', err.message), 'error');
  }
}

async function handleRestart() {
  await handleStop();
  setTimeout(handleStart, 1500);
}

function openProjectHome() {
  const url = 'https://github.com/itouakirai/wrapper';
  if ((isAndroidApp || typeof window.Android !== 'undefined') && window.Android && window.Android.openBrowser) {
    window.Android.openBrowser(url);
  } else {
    window.open(url, '_blank');
  }
}

const openServiceUrl = openProjectHome;

function copyServiceUrl() {
  const host = document.getElementById('cfg-host').value.trim() || '127.0.0.1';
  const port = document.getElementById('cfg-host-port').value.trim() || '12340';
  const url = `http://${host}:${port}`;
  navigator.clipboard.writeText(url).then(() => {
    alert(t('alert_copied', url));
  });
}

// UI State Toggles
function setRunningUI(regions = []) {
  isRunning = true;
  isStarting = false;
  currentRegions = regions || [];
  document.getElementById('btn-start').classList.add('hidden');
  document.getElementById('btn-stop').classList.remove('hidden');

  const dot = document.getElementById('status-dot');
  dot.className = 'status-dot running';
  document.getElementById('status-text').innerText = t('status_running');

  document.getElementById('metric-state').innerText = t('status_running');
  const stateBadge = document.getElementById('metric-state-badge');
  stateBadge.innerText = t('badge_online');
  stateBadge.className = 'badge badge-info';

  if (!serviceStartTime) serviceStartTime = Date.now();
  startUptimeTracker();

  if (currentRegions && currentRegions.length > 0) {
    document.getElementById('metric-regions').innerText = currentRegions.join(', ');
    document.getElementById('metric-regions-hint').innerText = `${currentRegions.length} ${t('hint_regions_active')}`;
  } else {
    document.getElementById('metric-regions').innerText = t('val_none');
    document.getElementById('metric-regions-hint').innerText = t('hint_regions_no_login');
  }
}

function setStoppedUI() {
  isRunning = false;
  isStarting = false;
  serviceStartTime = null;
  stopUptimeTracker();

  document.getElementById('btn-start').classList.remove('hidden');
  document.getElementById('btn-stop').classList.add('hidden');

  const dot = document.getElementById('status-dot');
  dot.className = 'status-dot stopped';
  document.getElementById('status-text').innerText = t('status_stopped');

  document.getElementById('metric-state').innerText = t('status_stopped');
  const stateBadge = document.getElementById('metric-state-badge');
  stateBadge.innerText = t('badge_inactive');
  stateBadge.className = 'badge';

  document.getElementById('metric-uptime').innerText = `${t('lbl_uptime')}: 0s`;
  document.getElementById('metric-regions').innerText = t('val_none');
  document.getElementById('metric-regions-hint').innerText = t('hint_regions_boot');
  document.getElementById('metric-latency').innerText = '-- ms';
  document.getElementById('metric-ping-badge').innerText = '--';
}

function setStartingUI() {
  isStarting = true;
  document.getElementById('btn-start').classList.add('hidden');
  document.getElementById('btn-stop').classList.remove('hidden');
  document.getElementById('status-dot').className = 'status-dot starting';
  document.getElementById('status-text').innerText = t('status_starting');
  document.getElementById('metric-state').innerText = t('status_booting');
}

function startUptimeTracker() {
  if (uptimeInterval) clearInterval(uptimeInterval);
  uptimeInterval = setInterval(() => {
    if (!serviceStartTime) return;
    const diff = Math.floor((Date.now() - serviceStartTime) / 1000);
    const m = Math.floor(diff / 60);
    const s = diff % 60;
    const text = m > 0 ? `${m}m ${s}s` : `${s}s`;
    document.getElementById('metric-uptime').innerText = `${t('lbl_uptime')}: ${text}`;
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
    if (typeof window.Android.hasLoginCache === 'function' && !hasCachedLogin) {
      try {
        hasCachedLogin = window.Android.hasLoginCache();
      } catch (e) {}
    }
  } else {
    try {
      const res = await fetch('/api/status');
      if (res.ok) {
        const data = await res.json();
        backendRunning = data.running;
        if (typeof data.hasLoginCache === 'boolean') {
          hasCachedLogin = data.hasLoginCache;
        }
      }
    } catch (e) {}
  }

  if (!backendRunning) {
    if (isRunning || isStarting) setStoppedUI();
    return;
  }

  // Ping the wrapper-lite HTTP port directly or via backend proxy
  const host = document.getElementById('cfg-host').value.trim() || '127.0.0.1';
  const port = document.getElementById('cfg-host-port').value.trim() || '12340';
  const pingTarget = host === '0.0.0.0' ? '127.0.0.1' : host;
  const startPing = Date.now();

  // On Android native app, invoke the native bridge probe (completely bypasses WebView file:// & CORS limitations)
  if (isAndroidApp && window.Android && window.Android.getServiceStatus) {
    try {
      const statusJson = window.Android.getServiceStatus(pingTarget, parseInt(port, 10));
      if (statusJson) {
        const data = JSON.parse(statusJson);
        if (data.online) {
          const latency = data.latency || 1;
          const regions = data.data && data.data.regions ? data.data.regions : [];
          setRunningUI(regions);
          document.getElementById('metric-latency').innerText = `${latency} ms`;
          const pingBadge = document.getElementById('metric-ping-badge');
          pingBadge.innerText = latency < 100 ? t('badge_good') : t('badge_normal');
          pingBadge.className = 'badge badge-info';
          return;
        }
      }
    } catch (e) {}
  }

  // Try direct fetch (for desktop or when browser allows direct access)
  if (!isAndroidApp) {
    try {
      const res = await fetch(`http://${pingTarget}:${port}/status`, { signal: AbortSignal.timeout(2000) });
      const latency = Date.now() - startPing;
      if (res.ok) {
        const data = await res.json();
        const regions = data.data && data.data.regions ? data.data.regions : [];
        setRunningUI(regions);
        document.getElementById('metric-latency').innerText = `${latency} ms`;
        const pingBadge = document.getElementById('metric-ping-badge');
        pingBadge.innerText = latency < 100 ? t('badge_good') : t('badge_normal');
        pingBadge.className = 'badge badge-info';
        return;
      }
    } catch (err) {
      // Direct fetch failed (e.g. CORS restrictions on older images or port not answering yet)
    }

    // Fallback to GUI backend service status probe (bypasses browser CORS completely)
    if (backendRunning) {
      try {
        const startProxy = Date.now();
        const res = await fetch('/api/service/status', { signal: AbortSignal.timeout(2500) });
        if (res.ok) {
          const data = await res.json();
          if (data.online) {
            const latency = data.latency || (Date.now() - startProxy);
            const regions = data.data && data.data.regions ? data.data.regions : [];
            setRunningUI(regions);
            document.getElementById('metric-latency').innerText = `${latency} ms`;
            const pingBadge = document.getElementById('metric-ping-badge');
            pingBadge.innerText = latency < 100 ? t('badge_good') : t('badge_normal');
            pingBadge.className = 'badge badge-info';
            return;
          }
        }
      } catch (e) {}
    }
  }

  // Port not answering yet (guest still booting)
  if (backendRunning) {
    setStartingUI();
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
    alert(t('alert_enter_credentials'));
    return;
  }

  submitBtn.disabled = true;
  progressBox.classList.remove('hidden');
  progressMsg.innerText = t('msg_logging_in');
  appendLog(t('log_login_initiating', username), 'info');

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
      hasCachedLogin = true;
      appendLog(t('log_login_succeeded'), 'run');
      alert(t('alert_login_succeeded'));
      detectPlatform();
    } else if (result.need2FA) {
      prompt2faModal();
    } else {
      hasCachedLogin = false;
      appendLog(t('log_login_failed', result.message || 'Authentication error'), 'error');
      alert(t('alert_login_failed', result.message || 'Check credentials'));
      detectPlatform();
    }
  } catch (err) {
    hasCachedLogin = false;
    appendLog(t('log_login_failed', err.message), 'error');
    alert(t('alert_login_failed', err.message));
    detectPlatform();
  } finally {
    submitBtn.disabled = false;
    progressBox.classList.add('hidden');
  }
}

function togglePasswordVisibility() {
  const pwdInput = document.getElementById('auth-password');
  const eyeIcon = document.getElementById('eye-icon');
  const eyeOffIcon = document.getElementById('eye-off-icon');
  if (!pwdInput) return;

  if (pwdInput.type === 'password') {
    pwdInput.type = 'text';
    if (eyeIcon) eyeIcon.classList.add('hidden');
    if (eyeOffIcon) eyeOffIcon.classList.remove('hidden');
  } else {
    pwdInput.type = 'password';
    if (eyeIcon) eyeIcon.classList.remove('hidden');
    if (eyeOffIcon) eyeOffIcon.classList.add('hidden');
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
    alert(t('alert_enter_2fa'));
    return;
  }
  close2faModal();
  document.getElementById('auth-2fa').value = code;
  handleLogin();
}

// QEMU Package Manager
async function checkQemuPackageStatus() {
  if (isAndroidApp || typeof window.Android !== 'undefined') {
    if (window.Android && window.Android.checkQemuStatus) {
      try {
        const raw = window.Android.checkQemuStatus();
        if (raw && raw !== '{}') {
          const data = JSON.parse(raw);
          updateQemuChecklist(data);
        }
      } catch (e) {
        console.warn('Android QEMU status parse error:', e);
      }
    }
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

window.checkQemuPackageStatus = checkQemuPackageStatus;

function updateQemuChecklist(data) {
  if (!data || Object.keys(data).length === 0) return;
  lastQemuData = data;

  const isAndroid = isAndroidApp || platformInfo.isAndroid || (typeof window.Android !== 'undefined');

  // On Android, the C++ host launcher (wrapper-lite-qemu) is not needed because the Android app runs QEMU directly via QemuRunner
  const launcherItem = document.getElementById('chk-launcher');
  if (launcherItem && isAndroid) {
    launcherItem.classList.add('hidden');
  }

  const items = [
    ...(!isAndroid ? [{ id: 'launcher', ok: !!data.launcher, name: 'wrapper-lite-qemu' }] : []),
    { id: 'kernel', ok: !!data.kernel, name: 'vmlinuz-lite-qemu' },
    { id: 'initramfs', ok: !!data.initramfs, name: 'lite-initramfs.cpio.gz' },
    { id: 'disk', ok: !!data.disk, name: 'data.img' },
    { id: 'qemubin', ok: !!data.qemuBin, name: 'qemu-system-x86_64' }
  ];

  let allReady = typeof data.allPresent === 'boolean'
    ? data.allPresent
    : (!!data.kernel && !!data.initramfs && !!data.disk && !!data.qemuBin && (isAndroid || !!data.launcher));

  items.forEach(item => {
    const icon = document.getElementById(`icon-${item.id}`);
    const detail = document.getElementById(`detail-${item.id}`);
    if (icon && detail) {
      if (item.ok) {
        icon.innerText = '✅';
        detail.innerText = t('status_present_verified');
        detail.style.color = 'var(--success-color)';
      } else {
        if (typeof data.allPresent !== 'boolean') {
          allReady = false;
        }
        icon.innerText = '❌';
        detail.innerText = t('status_missing_required');
        detail.style.color = 'var(--danger-color)';
      }
    }
  });

  const missingAlert = document.getElementById('qemu-missing-alert');
  if (missingAlert) {
    missingAlert.classList.toggle('hidden', allReady);
  }

  const pkgBadge = document.getElementById('qemu-package-badge');
  if (pkgBadge) {
    if (allReady) {
      pkgBadge.innerText = t('badge_qemu_ready');
      pkgBadge.className = 'badge badge-info';
    } else {
      pkgBadge.innerText = t('badge_qemu_incomplete');
      pkgBadge.className = 'badge badge-warning';
    }
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
  statusText.innerText = t('status_connecting_download');

  appendLog(t('log_pkg_downloading'), 'info');

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
          statusText.innerText = `${t('status_downloading_pkg')} (${pct}%)`;
          bytesText.innerText = `${(d.downloaded / 1048576).toFixed(1)} MB / ${(d.total / 1048576).toFixed(1)} MB`;
          speedText.innerText = `${(d.speed / 1024).toFixed(1)} KB/s`;
        } else if (d.status === 'extracting') {
          progressBar.style.width = '99%';
          pctText.innerText = '99%';
          statusText.innerText = t('status_extracting_pkg');
        } else if (d.status === 'done') {
          sse.close();
          progressBar.style.width = '100%';
          pctText.innerText = '100%';
          statusText.innerText = t('status_installed_pkg');
          btn.disabled = false;
          appendLog(t('log_pkg_installed'), 'run');
          setTimeout(() => {
            progressCard.classList.add('hidden');
            checkQemuPackageStatus();
          }, 2000);
        } else if (d.status === 'error') {
          sse.close();
          btn.disabled = false;
          statusText.innerText = `Error: ${d.message}`;
          appendLog(t('log_pkg_failed', d.message), 'error');
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
    appendLog(t('log_pkg_failed', err.message), 'error');
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

  statusEl.innerText = t('status_requesting');
  bodyEl.innerText = t('msg_waiting_response', url);

  try {
    const start = Date.now();
    let resText = '';
    let resStatus = 200;
    let resStatusText = 'OK';
    let latency = 0;

    if (isAndroidApp && window.Android && window.Android.proxyRequest) {
      const respStr = window.Android.proxyRequest(url, 'GET', '');
      latency = Date.now() - start;
      const respObj = JSON.parse(respStr);
      resStatus = respObj.status;
      resStatusText = respObj.statusText;
      resText = respObj.body;
    } else {
      let res;
      try {
        res = await fetch(url);
      } catch (directErr) {
        if (!isAndroidApp) {
          // Fallback through backend proxy if direct fetch fails (e.g. CORS)
          const proxyUrl = `/api/proxy?endpoint=${encodeURIComponent(ep + query)}`;
          res = await fetch(proxyUrl);
        } else {
          throw directErr;
        }
      }
      latency = Date.now() - start;
      resStatus = res.status;
      resStatusText = res.statusText;
      resText = await res.text();
    }

    statusEl.innerText = `${resStatus} ${resStatusText} (${latency}ms)`;

    try {
      const obj = JSON.parse(resText);
      bodyEl.innerHTML = `<code>${escapeHtml(JSON.stringify(obj, null, 2))}</code>`;
    } catch (e) {
      bodyEl.innerHTML = `<code>${escapeHtml(resText)}</code>`;
    }
  } catch (err) {
    statusEl.innerText = t('status_failed');
    bodyEl.innerHTML = `<code>Error: ${escapeHtml(err.message)}\nIs wrapper-lite running on ${targetHost}:${port}?</code>`;
  }
}

function safeCopyToClipboard(text, successMsg) {
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(() => {
      alert(successMsg);
    }).catch(() => {
      fallbackCopy(text, successMsg);
    });
  } else {
    fallbackCopy(text, successMsg);
  }
}

function fallbackCopy(text, successMsg) {
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    if (ok) {
      alert(successMsg);
      return;
    }
  } catch (e) {}
  alert(successMsg);
}

function copyTestResponse() {
  const text = document.getElementById('test-response-body').innerText;
  safeCopyToClipboard(text, t('alert_response_copied'));
}

const MAX_LOG_LINES = 500;

function createLogDiv(line, forcedClass = null) {
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
  return div;
}

function trimLogLines(container, max) {
  while (container.childNodes.length > max) {
    container.removeChild(container.firstChild);
  }
}

// Live Logs Terminal
function initLogStream() {
  if (isAndroidApp) {
    // Android calls window.onAndroidLogBatch(lines) or window.onAndroidLogEntry(line)
    window.onAndroidLogBatch = (lines) => {
      appendLogBatch(lines);
    };
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

function appendLogBatch(lines) {
  if (!Array.isArray(lines) || lines.length === 0) return;
  const terminal = document.getElementById('logs-terminal');
  const dashTerminal = document.getElementById('dashboard-logs-terminal');
  if (!terminal && !dashTerminal) return;

  const validLines = lines.filter(l => typeof l === 'string' && !l.includes('request: GET /status'));
  if (validLines.length === 0) return;

  const termFrag = terminal ? document.createDocumentFragment() : null;
  const dashFrag = dashTerminal ? document.createDocumentFragment() : null;

  for (const line of validLines) {
    if (termFrag) termFrag.appendChild(createLogDiv(line));
    if (dashFrag) dashFrag.appendChild(createLogDiv(line));
    logsCount++;
  }

  if (terminal && termFrag) {
    terminal.appendChild(termFrag);
    trimLogLines(terminal, MAX_LOG_LINES);
    const chkAutoscroll = document.getElementById('chk-autoscroll');
    if (chkAutoscroll && chkAutoscroll.checked) {
      terminal.scrollTop = terminal.scrollHeight;
    }
  }

  if (dashTerminal && dashFrag) {
    dashTerminal.appendChild(dashFrag);
    trimLogLines(dashTerminal, 100);
    const chkDashAutoscroll = document.getElementById('chk-dashboard-autoscroll');
    if (chkDashAutoscroll && chkDashAutoscroll.checked) {
      dashTerminal.scrollTop = dashTerminal.scrollHeight;
    }
  }

  const badge = document.getElementById('logs-count-badge');
  if (badge) badge.innerText = `${logsCount} ${t('unit_lines')}`;
  const dashBadge = document.getElementById('dashboard-logs-count-badge');
  if (dashBadge) dashBadge.innerText = `${logsCount} ${t('unit_lines')}`;
}

function appendLog(line, forcedClass = null) {
  if (forcedClass) {
    const terminal = document.getElementById('logs-terminal');
    const dashTerminal = document.getElementById('dashboard-logs-terminal');
    if (terminal) {
      terminal.appendChild(createLogDiv(line, forcedClass));
      trimLogLines(terminal, MAX_LOG_LINES);
      const chkAutoscroll = document.getElementById('chk-autoscroll');
      if (chkAutoscroll && chkAutoscroll.checked) terminal.scrollTop = terminal.scrollHeight;
    }
    if (dashTerminal) {
      dashTerminal.appendChild(createLogDiv(line, forcedClass));
      trimLogLines(dashTerminal, 100);
      const chkDashAutoscroll = document.getElementById('chk-dashboard-autoscroll');
      if (chkDashAutoscroll && chkDashAutoscroll.checked) dashTerminal.scrollTop = dashTerminal.scrollHeight;
    }
    logsCount++;
    const badge = document.getElementById('logs-count-badge');
    if (badge) badge.innerText = `${logsCount} ${t('unit_lines')}`;
    const dashBadge = document.getElementById('dashboard-logs-count-badge');
    if (dashBadge) dashBadge.innerText = `${logsCount} ${t('unit_lines')}`;
  } else {
    appendLogBatch([line]);
  }
}

function clearLogs() {
  const terminal = document.getElementById('logs-terminal');
  if (terminal) terminal.innerHTML = '';
  const dashTerminal = document.getElementById('dashboard-logs-terminal');
  if (dashTerminal) dashTerminal.innerHTML = '';
  logsCount = 0;
  const badge = document.getElementById('logs-count-badge');
  if (badge) badge.innerText = `0 ${t('unit_lines')}`;
  const dashBadge = document.getElementById('dashboard-logs-count-badge');
  if (dashBadge) dashBadge.innerText = `0 ${t('unit_lines')}`;
}

function copyAllLogs() {
  const terminal = document.getElementById('logs-terminal') || document.getElementById('dashboard-logs-terminal');
  if (!terminal) return;
  safeCopyToClipboard(terminal.innerText, t('alert_logs_copied'));
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
      if (data.regions && data.regions.length > 0) {
        setRunningUI(data.regions);
      } else {
        checkServiceHealth();
      }
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
  if (statusText) statusText.innerText = status || `${t('status_downloading_pkg')}...`;
  if (speedText) speedText.innerText = speed || '';

  if (pct >= 100) {
    if (btn) btn.disabled = false;
    appendLog(t('log_pkg_installed'), 'run');
    setTimeout(() => {
      if (progressCard) progressCard.classList.add('hidden');
      checkQemuPackageStatus();
    }, 2000);
  } else if (status && status.startsWith('Error:')) {
    if (btn) btn.disabled = false;
    appendLog(t('log_pkg_failed', status), 'error');
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
    hasCachedLogin = true;
    appendLog(t('log_login_succeeded'), 'run');
    alert(t('alert_login_succeeded'));
    detectPlatform();
  } else if (result.need2FA) {
    prompt2faModal();
  } else {
    hasCachedLogin = false;
    appendLog(t('log_login_failed', result.message || 'Authentication error'), 'error');
    alert(t('alert_login_failed', result.message || 'Check credentials'));
    detectPlatform();
  }
};
