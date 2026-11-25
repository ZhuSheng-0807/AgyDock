const tauriInvoke =
  typeof window !== 'undefined' && typeof window.__TAURI__?.core?.invoke === 'function'
    ? window.__TAURI__.core.invoke
    : null;

const tauriListen =
  typeof window !== 'undefined' && typeof window.__TAURI__?.event?.listen === 'function'
    ? window.__TAURI__.event.listen
    : null;

function escapeHtml(str) {
  return String(str ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function nowTimeStr() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function normPathKey(p) {
  return String(p || '')
    .trim()
    .replace(/\//g, '\\')
    .replace(/\\+$/, '')
    .toLowerCase();
}

function slugifyToolId(raw) {
  const slug = String(raw || '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || 'tool';
}

function isGeminiDesktopTool(tool) {
  if (!tool) return false;
  if (tool.id === 'gemini-desktop') return true;
  const key = normPathKey(tool.exePath);
  return key.includes('\\google\\gemini\\') && key.endsWith('\\gemini.exe');
}

export function initProxyModule() {
  const els = {
    launchBtn: document.getElementById('proxyLaunchBtn'),
    auditBtn: document.getElementById('proxyAuditBtn'),
    settingsBtn: document.getElementById('proxySettingsBtn'),
    refreshBtn: document.getElementById('proxyRefreshBtn'),

    warnBanner: document.getElementById('proxyWarnBanner'),
    globalWarnBanner: document.getElementById('proxyGlobalWarnBanner'),
    confirmBanner: document.getElementById('proxyConfirmBanner'),
    confirmText: document.getElementById('proxyConfirmText'),
    confirmOkBtn: document.getElementById('proxyConfirmOkBtn'),
    confirmCancelBtn: document.getElementById('proxyConfirmCancelBtn'),

    auditBar: document.getElementById('proxyAuditBar'),
    auditFill: document.getElementById('proxyAuditFill'),
    auditLabel: document.getElementById('proxyAuditLabel'),

    statusDot: document.getElementById('proxyStatusDot'),
    headline: document.getElementById('proxyHeadline'),
    headlineSub: document.getElementById('proxyHeadlineSub'),
    envValue: document.getElementById('proxyEnvValue'),
    envSub: document.getElementById('proxyEnvSub'),
    procValue: document.getElementById('proxyProcValue'),
    procSub: document.getElementById('proxyProcSub'),
    connValue: document.getElementById('proxyConnValue'),
    connSub: document.getElementById('proxyConnSub'),

    settingsPanel: document.getElementById('proxySettingsPanel'),
    inputHost: document.getElementById('proxyInputHost'),
    chkAutoPort: document.getElementById('proxyChkAutoPort'),
    inputPort: document.getElementById('proxyInputPort'),
    chkBypassChina: document.getElementById('proxyChkBypassChina'),
    inputCustomBypass: document.getElementById('proxyInputCustomBypass'),
    btnClearBypass: document.getElementById('proxyBtnClearBypass'),
    chkAutoClash: document.getElementById('proxyChkAutoClash'),
    chkDisableQuic: document.getElementById('proxyChkDisableQuic'),
    chkAutoRefresh: document.getElementById('proxyChkAutoRefresh'),
    inputAgyPath: document.getElementById('proxyInputAgyPath'),
    btnBrowseAgy: document.getElementById('proxyBtnBrowseAgy'),
    inputCliPath: document.getElementById('proxyInputCliPath'),
    btnBrowseCli: document.getElementById('proxyBtnBrowseCli'),
    btnDetectCli: document.getElementById('proxyBtnDetectCli'),
    inputClashPath: document.getElementById('proxyInputClashPath'),
    btnBrowseClash: document.getElementById('proxyBtnBrowseClash'),
    inputWorkspace: document.getElementById('proxyInputWorkspace'),
    btnBrowseWorkspace: document.getElementById('proxyBtnBrowseWorkspace'),
    btnClearWorkspace: document.getElementById('proxyBtnClearWorkspace'),

    cliStatusTag: document.getElementById('proxyCliStatusTag'),
    shimStatusTag: document.getElementById('proxyShimStatusTag'),
    btnCliTerminal: document.getElementById('proxyBtnCliTerminal'),
    btnTestCli: document.getElementById('proxyBtnTestCli'),
    btnToggleShim: document.getElementById('proxyBtnToggleShim'),
    btnSyncShim: document.getElementById('proxyBtnSyncShim'),
    btnCopySnippets: document.getElementById('proxyBtnCopySnippets'),
    btnStopCli: document.getElementById('proxyBtnStopCli'),
    btnDetectTools: document.getElementById('proxyBtnDetectTools'),
    btnAddTool: document.getElementById('proxyBtnAddTool'),
    toolForm: document.getElementById('proxyToolForm'),
    toolName: document.getElementById('proxyToolName'),
    toolIsChromium: document.getElementById('proxyToolIsChromium'),
    toolMonitor: document.getElementById('proxyToolMonitor'),
    toolExePath: document.getElementById('proxyToolExePath'),
    toolBrowseExe: document.getElementById('proxyToolBrowseExe'),
    toolArgs: document.getElementById('proxyToolArgs'),
    toolWorkDir: document.getElementById('proxyToolWorkDir'),
    toolBrowseDir: document.getElementById('proxyToolBrowseDir'),
    toolSaveBtn: document.getElementById('proxyToolSaveBtn'),
    toolCancelBtn: document.getElementById('proxyToolCancelBtn'),
    cliToolsBody: document.getElementById('proxyCliToolsBody'),

    subViewControl: document.getElementById('proxySubViewControl'),
    tabConnBtn: document.getElementById('proxyTabConnBtn'),
    chkShowIpc: document.getElementById('proxyChkShowIpc'),
    connTools: document.getElementById('proxyConnTools'),
    logTools: document.getElementById('proxyLogTools'),

    btnStartClash: document.getElementById('proxyBtnStartClash'),
    btnFlushDns: document.getElementById('proxyBtnFlushDns'),
    btnDryRun: document.getElementById('proxyBtnDryRun'),
    btnStopAgy: document.getElementById('proxyBtnStopAgy'),
    btnOpenReport: document.getElementById('proxyBtnOpenReport'),
    btnCopyLog: document.getElementById('proxyBtnCopyLog'),
    btnClearLog: document.getElementById('proxyBtnClearLog'),

    connView: document.getElementById('proxyConnView'),
    connTableBody: document.getElementById('proxyConnTableBody'),
    logView: document.getElementById('proxyLogView'),
    logPre: document.getElementById('proxyLogPre'),

    footerLeft: document.getElementById('proxyFooterLeft'),
    footerRight: document.getElementById('proxyFooterRight'),
  };

  const state = {
    active: false,
    subView: 'connections',
    settingsExpanded: false,
    busy: false,
    auditing: false,
    refreshing: false,
    autoRefresh: true,
    settings: null,
    cliTools: [],
    cliToolsInitialized: false,
    builtinToolsDiscovered: [],
    editingToolId: null,
    snapshot: null,
    displayedConnections: [],
    logs: [],
    pendingConfirmAction: null,
    timerId: null,
    unlistenAudit: null,
  };

  function appendLog(msg) {
    const lines = String(msg ?? '').split(/\r?\n/);
    const ts = nowTimeStr();
    for (const line of lines) {
      if (!line.trim()) continue;
      state.logs.push(`[${ts}] ${line}`);
    }
    if (state.logs.length > 400) {
      state.logs = state.logs.slice(-400);
    }
    els.logPre.textContent = state.logs.join('\n');
    els.logPre.scrollTop = els.logPre.scrollHeight;
  }

  function showConfirm(message, onConfirm) {
    state.pendingConfirmAction = onConfirm;
    els.confirmText.textContent = message;
    els.confirmBanner.classList.remove('hidden');
  }

  function hideConfirm() {
    state.pendingConfirmAction = null;
    els.confirmBanner.classList.add('hidden');
  }

  function setBusy(busy) {
    state.busy = busy;
    const disabled = busy || state.auditing;
    [
      els.launchBtn,
      els.refreshBtn,
      els.btnStartClash,
      els.btnFlushDns,
      els.btnDryRun,
      els.btnStopAgy,
      els.btnCliTerminal,
      els.btnTestCli,
      els.btnToggleShim,
      els.btnSyncShim,
      els.btnStopCli,
      els.btnDetectCli,
      els.btnDetectTools,
    ]
      .filter(Boolean)
      .forEach((btn) => {
        btn.disabled = disabled;
      });
    if (els.cliToolsBody) {
      els.cliToolsBody
        .querySelectorAll('.cli-tool-launch-btn, .cli-tool-stop-btn')
        .forEach((btn) => {
          btn.disabled = disabled;
        });
    }
  }

  function setSubView(view) {
    state.subView = view;
    els.subViewControl.querySelectorAll('.seg-btn').forEach((b) => {
      b.classList.toggle('active', b.dataset.subview === view);
    });
    const isConn = view === 'connections';
    els.connView.classList.toggle('hidden', !isConn);
    els.connTools.classList.toggle('hidden', !isConn);
    els.logView.classList.toggle('hidden', isConn);
    els.logTools.classList.toggle('hidden', isConn);
  }

  function renderCliTools() {
    if (!els.cliToolsBody) return;
    const counts = state.snapshot?.toolRunningCounts || {};
    const disabledAttr = state.busy || state.auditing ? 'disabled' : '';

    if (!state.cliTools.length) {
      els.cliToolsBody.innerHTML = `
        <tr class="empty-row">
          <td colspan="4">
            <span>当前尚未配置外部应用（如 Gemini Desktop、T3 Code）</span>
            <button type="button" class="btn" id="proxyBtnAutoDetectTools" style="margin-left: 10px;">自动探测应用</button>
          </td>
        </tr>
      `;
      return;
    }

    const snap = state.snapshot;
    const sysOn = Boolean(snap?.systemProxyEnabled);
    const tunOn = Boolean(snap?.tunEnabled);
    const managedSys = Boolean(snap?.geminiSystemProxyManaged);
    const geminiHasDirect = (state.displayedConnections || []).some(
      (c) =>
        c.category === 'publicDirect' &&
        String(c.processName || '').toLowerCase() === 'gemini'
    );

    els.cliToolsBody.innerHTML = state.cliTools
      .map((tool) => {
        const running = counts[tool.id] || 0;
        const isGemini = isGeminiDesktopTool(tool);
        let modeHtml;
        let hintHtml = '';
        if (isGemini) {
          if (managedSys && sysOn) {
            modeHtml =
              '<span class="tag tag-ok mono" title="已自动开启 Windows 系统代理，退出 Gemini 后将自动关闭">系统代理 · 自动托管</span>';
            hintHtml =
              '<div class="cli-tool-note cli-tool-note-muted">已自动开启系统代理，关闭 Gemini 后将自动恢复</div>';
          } else if (sysOn) {
            modeHtml =
              '<span class="tag tag-ok mono" title="Windows 系统代理已开启，Gemini 页面流量正通过代理">系统代理 · 已生效</span>';
          } else if (tunOn) {
            modeHtml =
              '<span class="tag tag-ok mono" title="TUN 模式已开启，Gemini 流量正通过 TUN 代理">TUN · 已生效</span>';
          } else if (running > 0 && geminiHasDirect) {
            modeHtml =
              '<span class="tag tag-err mono" title="未开启系统代理或 TUN，检测到 Gemini 正在公网直连">未代理 · 直连中</span>';
            hintHtml =
              '<div class="cli-tool-note">未开启系统代理或 TUN，点击「重启」可自动开启系统代理</div>';
          } else if (running > 0) {
            modeHtml =
              '<span class="tag tag-warn mono" title="未开启系统代理或 TUN，点击「重启」可自动开启系统代理">未开启系统代理</span>';
            hintHtml =
              '<div class="cli-tool-note">未开启系统代理或 TUN，点击「重启」可自动开启系统代理</div>';
          } else {
            modeHtml =
              '<span class="tag mono" title="启动时自动开启 Windows 系统代理，关闭 Gemini 后自动关闭">自动系统代理</span>';
            hintHtml =
              '<div class="cli-tool-note cli-tool-note-muted">启动时自动开启系统代理，关闭后自动恢复</div>';
          }
        } else {
          modeHtml = `<span class="tag mono">${tool.isChromium ? 'Chromium + Env' : '仅环境变量'}</span>`;
        }
        const statusHtml = !tool.monitor
          ? '<span class="tag">未监控</span>'
          : running > 0
            ? `<span class="tag tag-ok">运行中 x${running}</span>`
            : '<span class="tag">未运行</span>';
        const launchLabel = isGemini
          ? running > 0
            ? '重启'
            : '启动'
          : running > 0
            ? '隔离重启'
            : '隔离启动';
        const launchClass = running > 0 ? 'btn' : 'btn btn-primary';
        const argsTag = tool.args
          ? `<span class="tag mono">${escapeHtml(tool.args)}</span>`
          : '';
        const dirTag = tool.workingDir
          ? `<span class="tag mono" title="${escapeHtml(tool.workingDir)}">自定义目录</span>`
          : '';
        const idAttr = escapeHtml(tool.id);

        const stopDisabled = state.busy || state.auditing || running === 0 ? 'disabled' : '';
        const stopTitle = running > 0 ? '结束该工具进程' : '当前未运行';

        return `
          <tr>
            <td>
              <div class="cli-tool-meta">
                <div class="cli-tool-title-row">
                  <span class="cli-tool-name">${escapeHtml(tool.name)}</span>
                  ${argsTag}
                  ${dirTag}
                </div>
                <div class="cli-tool-path mono" title="${escapeHtml(tool.exePath)}">${escapeHtml(tool.exePath)}</div>
                ${hintHtml}
              </div>
            </td>
            <td>${modeHtml}</td>
            <td>${statusHtml}</td>
            <td>
              <div class="cli-tool-actions">
                <button type="button" class="${launchClass} cli-tool-launch-btn" data-tool-id="${idAttr}" ${disabledAttr}>${launchLabel}</button>
                <button type="button" class="btn cli-tool-stop-btn" data-tool-id="${idAttr}" ${stopDisabled} title="${stopTitle}">结束</button>
                <button type="button" class="btn cli-tool-edit-btn" data-tool-id="${idAttr}">编辑</button>
                <button type="button" class="btn btn-danger-subtle cli-tool-del-btn" data-tool-id="${idAttr}">删除</button>
              </div>
            </td>
          </tr>
        `;
      })
      .join('');
  }

  function renderConnections() {
    const showIpc = Boolean(els.chkShowIpc.checked);
    const filtered = showIpc
      ? state.displayedConnections
      : state.displayedConnections.filter((c) => c.category !== 'localIpc');

    els.tabConnBtn.textContent = `连接明细 (${filtered.length})`;

    if (!filtered.length) {
      els.connTableBody.innerHTML =
        '<tr class="empty-row"><td colspan="6">当前没有可显示的外部连接（在 Antigravity、Gemini Desktop、agy CLI 或 T3 Code 发一条消息即可看到实时套接字）</td></tr>';
      return;
    }

    els.connTableBody.innerHTML = filtered
      .map((c) => {
        const badgeClass =
          c.category === 'proxied'
            ? 'tag tag-ok'
            : c.category === 'tunFakeIp'
              ? 'tag tag-warn'
              : c.category === 'publicDirect'
                ? 'tag tag-err'
                : 'tag';
        return `
          <tr>
            <td><span class="${badgeClass}">${escapeHtml(c.categoryLabel)}</span></td>
            <td class="mono">${escapeHtml(c.processName)}</td>
            <td class="num">${c.pid}</td>
            <td class="mono">${escapeHtml(c.localEndpoint)}</td>
            <td class="mono">${escapeHtml(c.remoteEndpoint)}</td>
            <td class="mono nowrap">${escapeHtml(c.state)} · ${escapeHtml(c.seenAt)}</td>
          </tr>
        `;
      })
      .join('');
  }

  function applySnapshot(snap, overrideConnections = null, overrideCounts = null) {
    if (!snap) return;
    state.snapshot = snap;
    state.displayedConnections = overrideConnections || snap.connections || [];

    const proxied = overrideCounts ? overrideCounts.proxied : snap.proxiedCount;
    const tun = overrideCounts ? overrideCounts.tun : snap.tunCount;
    const direct = overrideCounts ? overrideCounts.direct : snap.directCount;
    const agyProcs = (snap.antigravityProcCount || 0) + (snap.languageServerProcCount || 0);
    const cliProcs = snap.cliProcCount || 0;
    const toolProcs = snap.toolProcCount || 0;
    const totalProcs = agyProcs + cliProcs + toolProcs;

    const bypassEnabled = Boolean(
      state.settings?.bypassChinaAndLan || state.settings?.customBypassDomains?.trim()
    );
    const coreDirect = state.displayedConnections.some((c) => {
      if (c.category !== 'publicDirect') return false;
      const procLower = String(c.processName || '').toLowerCase();
      return (
        procLower.startsWith('language_server') ||
        procLower === 'agy' ||
        procLower === 'antigravity-cli'
      );
    });

    // Status headline & dot
    els.statusDot.className = 'status-dot';
    if (!snap.isProxyListening) {
      els.statusDot.classList.add('err');
      els.headline.textContent = '代理端口未监听';
    } else if (totalProcs === 0) {
      els.statusDot.classList.add('muted');
      els.headline.textContent = 'Antigravity 与 CLI 未运行';
    } else if (coreDirect || (!bypassEnabled && direct > 0)) {
      els.statusDot.classList.add('err');
      els.headline.textContent = '检测到公网直连';
    } else if (tun > 0) {
      els.statusDot.classList.add('warn');
      els.headline.textContent = '由 TUN 网卡接管';
    } else if (proxied > 0 && direct > 0 && bypassEnabled) {
      els.headline.textContent = '隔离代理运行中 (含直连绕过)';
    } else if (proxied > 0) {
      els.headline.textContent = '隔离代理运行中';
    } else {
      els.headline.textContent = '运行中（空闲）';
    }

    const modeSuffix = snap.clashMode && snap.clashMode !== '未知' ? ` (${snap.clashMode})` : '';
    const bypassTag = bypassEnabled ? ' · 国内直连' : '';
    const headlineShort = `${snap.proxyHost}:${snap.resolvedPort}${modeSuffix}${bypassTag}`;
    els.headlineSub.textContent = headlineShort;
    els.headlineSub.title = `${headlineShort} · ${snap.portSource}`;

    els.launchBtn.textContent = agyProcs > 0 ? '重启 Antigravity' : '启动 Antigravity';

    // System env cell
    const sysText = snap.systemProxyEnabled ? '系统代理 开' : '系统代理 关';
    const tunText = snap.tunEnabled ? 'TUN 开' : 'TUN 关';
    els.envValue.textContent = `${sysText} / ${tunText}`;
    if (snap.geminiSystemProxyManaged && snap.systemProxyEnabled && !snap.tunEnabled) {
      els.envSub.textContent = `Gemini 自动托管中 (${snap.systemProxyServer || '已开启'})，退出后自动恢复`;
    } else if (snap.systemProxyEnabled || snap.tunEnabled) {
      els.envSub.textContent = snap.tunEnabled
        ? `网卡: ${snap.tunAdapterText || 'TUN Active'}`
        : `系统代理: ${snap.systemProxyServer || '已开启'}`;
    } else {
      els.envSub.textContent = '不改系统代理，仅隔离已托管应用与 CLI';
    }

    const showSysWarn =
      Boolean(snap.tunEnabled) ||
      (Boolean(snap.systemProxyEnabled) && !snap.geminiSystemProxyManaged);
    els.warnBanner.classList.toggle('hidden', !showSysWarn);
    if (els.globalWarnBanner) {
      const isGlobalMode = String(snap.clashMode || '').trim().toLowerCase() === 'global';
      els.globalWarnBanner.classList.toggle('hidden', !isGlobalMode);
    }

    // Process cell
    els.procValue.textContent = `${totalProcs} 进程`;
    const procParts = [];
    if (snap.antigravityProcCount > 0) procParts.push(`App x${snap.antigravityProcCount}`);
    if (snap.languageServerProcCount > 0) procParts.push(`LS x${snap.languageServerProcCount}`);
    if (cliProcs > 0) procParts.push(`CLI x${cliProcs}`);
    if (toolProcs > 0) procParts.push(`应用 x${toolProcs}`);
    procParts.push(`Clash ${snap.isClashRunning ? '开' : '关'}`);
    els.procSub.textContent = procParts.join(' · ');
    els.procSub.title = `App x${snap.antigravityProcCount} · LS x${snap.languageServerProcCount} · CLI x${cliProcs} · 应用 x${toolProcs} · Clash ${snap.isClashRunning ? '运行中' : '未运行'}`;

    // Connections count cell
    els.connValue.innerHTML = `
      <span class="${proxied > 0 ? 'tone-ok' : 'tone-muted'}">${proxied}</span><span class="slash">/</span><span class="${tun > 0 ? 'tone-warn' : 'tone-muted'}">${tun}</span><span class="slash">/</span><span class="${direct > 0 ? 'tone-err' : 'tone-muted'}">${direct}</span>
    `;
    els.connSub.textContent = '走代理 / TUN / 公网直连';

    // CLI & Shim status tags
    const shim = snap.cliShim || {};
    if (els.cliStatusTag) {
      if (cliProcs > 0) {
        els.cliStatusTag.textContent = `agy 运行中 x${cliProcs}`;
        els.cliStatusTag.className = 'tag mono tag-ok';
      } else if (shim.cliDetected) {
        els.cliStatusTag.textContent = 'CLI 已就绪';
        els.cliStatusTag.className = 'tag mono';
      } else {
        els.cliStatusTag.textContent = '未检测到 CLI';
        els.cliStatusTag.className = 'tag mono tag-warn';
      }
      els.cliStatusTag.title = shim.cliPath ? `CLI: ${shim.cliPath}` : '未检测到 agy.exe';
    }
    if (els.shimStatusTag) {
      if (shim.enabled && shim.outOfSync) {
        els.shimStatusTag.textContent = 'Shim 待同步';
        els.shimStatusTag.className = 'tag mono tag-warn';
      } else if (shim.enabled) {
        els.shimStatusTag.textContent = 'Shim 已启用';
        els.shimStatusTag.className = 'tag mono tag-ok';
      } else {
        els.shimStatusTag.textContent = 'Shim 未启用';
        els.shimStatusTag.className = 'tag mono';
      }
      els.shimStatusTag.title = shim.shimDir ? `Shim 目录: ${shim.shimDir}` : '';
    }
    if (els.btnToggleShim) {
      els.btnToggleShim.textContent = shim.enabled ? '停用 CLI Shim' : '启用 CLI Shim';
    }
    if (els.btnSyncShim) {
      els.btnSyncShim.classList.toggle('hidden', !(shim.enabled && shim.outOfSync));
    }
    if (els.btnStopCli) {
      els.btnStopCli.classList.toggle('hidden', cliProcs === 0);
    }

    renderCliTools();
    renderConnections();

    const ws = state.settings?.workspacePath?.trim();
    els.footerLeft.textContent = `端口来源: ${snap.portSource}${ws ? ` · 启动目录: ${ws}` : ''}`;
    els.footerRight.textContent = `更新于 ${snap.capturedAt}`;
  }

  function applySettingsToForm(s) {
    state.settings = s;
    els.inputHost.value = s.proxyHost || '127.0.0.1';
    els.chkAutoPort.checked = Boolean(s.autoDetectPort);
    els.inputPort.value = String(s.customPort || 7897);
    els.inputPort.disabled = Boolean(s.autoDetectPort);
    if (els.chkBypassChina) els.chkBypassChina.checked = s.bypassChinaAndLan !== false;
    if (els.inputCustomBypass) els.inputCustomBypass.value = s.customBypassDomains || '';
    els.chkAutoClash.checked = Boolean(s.autoStartClash);
    els.chkDisableQuic.checked = Boolean(s.disableQuic);
    els.chkShowIpc.checked = Boolean(s.showLoopbackIpc);
    els.inputAgyPath.value = s.antigravityPath || '';
    if (els.inputCliPath) els.inputCliPath.value = s.cliPath || '';
    els.inputClashPath.value = s.clashVergePath || '';
    els.inputWorkspace.value = s.workspacePath || '';
    state.cliTools = Array.isArray(s.cliTools) ? s.cliTools.map((t) => ({ ...t })) : [];
    state.cliToolsInitialized = Boolean(s.cliToolsInitialized);
    state.builtinToolsDiscovered = Array.isArray(s.builtinToolsDiscovered)
      ? [...s.builtinToolsDiscovered]
      : [];
    renderCliTools();
  }

  function collectSettingsFromForm() {
    const portNum = parseInt(els.inputPort.value.trim(), 10);
    return {
      proxyHost: els.inputHost.value.trim() || '127.0.0.1',
      autoDetectPort: Boolean(els.chkAutoPort.checked),
      customPort: Number.isFinite(portNum) && portNum > 0 && portNum <= 65535 ? portNum : 7897,
      antigravityPath: els.inputAgyPath.value.trim(),
      clashVergePath: els.inputClashPath.value.trim(),
      workspacePath: els.inputWorkspace.value.trim(),
      autoStartClash: Boolean(els.chkAutoClash.checked),
      disableQuic: Boolean(els.chkDisableQuic.checked),
      showLoopbackIpc: Boolean(els.chkShowIpc.checked),
      bypassChinaAndLan: els.chkBypassChina ? Boolean(els.chkBypassChina.checked) : true,
      customBypassDomains: els.inputCustomBypass ? els.inputCustomBypass.value.trim() : '',
      cliPath: els.inputCliPath ? els.inputCliPath.value.trim() : '',
      cliTools: state.cliTools,
      cliToolsInitialized: state.cliToolsInitialized,
      builtinToolsDiscovered: state.builtinToolsDiscovered,
    };
  }

  let saveTimer = null;
  async function persistSettings(immediate = false) {
    if (!tauriInvoke) return;
    const doSave = async () => {
      try {
        const next = collectSettingsFromForm();
        const saved = await tauriInvoke('proxy_save_settings', { settings: next });
        state.settings = saved;
        state.cliTools = Array.isArray(saved.cliTools) ? saved.cliTools.map((t) => ({ ...t })) : [];
        state.cliToolsInitialized = Boolean(saved.cliToolsInitialized);
        state.builtinToolsDiscovered = Array.isArray(saved.builtinToolsDiscovered)
          ? [...saved.builtinToolsDiscovered]
          : [];
        if (els.inputCliPath && document.activeElement !== els.inputCliPath) {
          els.inputCliPath.value = saved.cliPath || '';
        }
        renderCliTools();
      } catch (err) {
        appendLog(`[!] 保存设置失败: ${err}`);
      }
    };
    if (immediate) {
      clearTimeout(saveTimer);
      await doSave();
    } else {
      clearTimeout(saveTimer);
      saveTimer = setTimeout(doSave, 250);
    }
  }

  async function refreshSnapshot() {
    if (!tauriInvoke || state.refreshing || state.auditing) return;
    state.refreshing = true;
    try {
      const snap = await tauriInvoke('proxy_get_snapshot');
      applySnapshot(snap);
    } catch (err) {
      appendLog(`[!] 刷新状态失败: ${err}`);
    } finally {
      state.refreshing = false;
    }
  }

  function startAutoRefresh() {
    stopAutoRefresh();
    if (!state.active || !state.autoRefresh || !tauriInvoke) return;
    state.timerId = setInterval(() => {
      if (state.active && document.visibilityState === 'visible' && !state.auditing) {
        refreshSnapshot();
      }
    }, 1500);
  }

  function stopAutoRefresh() {
    if (state.timerId) {
      clearInterval(state.timerId);
      state.timerId = null;
    }
  }

  async function runLaunch(forceRestart) {
    if (!tauriInvoke) return;
    hideConfirm();
    await persistSettings(true);
    setBusy(true);
    try {
      const res = await tauriInvoke('proxy_launch_antigravity', { forceRestart });
      for (const l of res.logs || []) appendLog(l);
      if (res.needConfirm) {
        showConfirm(
          `检测到当前有 ${res.processCount} 个 Antigravity / language_server 进程正在运行。必须先关闭旧进程才能让隔离代理环境变量生效，确认关闭并重启？`,
          () => runLaunch(true)
        );
        return;
      }
      if (!res.ok && res.error) {
        appendLog(`[X] ${res.error}`);
        setSubView('logs');
      } else if (res.ok) {
        setTimeout(() => refreshSnapshot(), 1100);
      }
    } catch (err) {
      appendLog(`[X] 启动异常: ${err}`);
      setSubView('logs');
    } finally {
      setBusy(false);
    }
  }

  async function runLaunchTool(toolId, forceRestart) {
    if (!tauriInvoke) return;
    const tool = state.cliTools.find((t) => t.id === toolId);
    if (!tool) return;
    hideConfirm();
    await persistSettings(true);
    setBusy(true);
    try {
      const res = await tauriInvoke('proxy_launch_cli_tool', { toolId, forceRestart });
      for (const l of res.logs || []) appendLog(l);
      if (res.needConfirm) {
        const confirmMsg = isGeminiDesktopTool(tool)
          ? `检测到 ${tool.name} 当前有 ${res.processCount} 个相关进程正在运行，确认关闭并重新启动？`
          : `检测到 ${tool.name} 当前有 ${res.processCount} 个相关进程正在运行。必须重启才能让隔离代理生效，确认关闭并重启？`;
        showConfirm(confirmMsg, () => runLaunchTool(toolId, true));
        return;
      }
      if (!res.ok && res.error) {
        appendLog(`[X] ${res.error}`);
        setSubView('logs');
      } else if (res.ok) {
        setTimeout(() => refreshSnapshot(), 900);
      }
    } catch (err) {
      appendLog(`[X] 启动 ${tool.name} 异常: ${err}`);
      setSubView('logs');
    } finally {
      setBusy(false);
    }
  }

  async function runStopTool(toolId) {
    if (!tauriInvoke || state.busy) return;
    const tool = state.cliTools.find((t) => t.id === toolId);
    if (!tool) return;
    const running = state.snapshot?.toolRunningCounts?.[toolId] || 0;
    const doStop = async () => {
      setBusy(true);
      try {
        const res = await tauriInvoke('proxy_stop_cli_tool', { toolId });
        for (const l of res.logs || []) appendLog(l);
        await refreshSnapshot();
      } catch (err) {
        appendLog(`[X] 结束 ${tool.name} 失败: ${err}`);
      } finally {
        setBusy(false);
      }
    };
    if (running > 0) {
      showConfirm(
        `确定要结束 ${tool.name} 及其 ${running} 个相关进程吗？`,
        doStop
      );
    } else {
      await doStop();
    }
  }

  function openToolForm(tool = null) {
    if (!els.toolForm) return;
    state.editingToolId = tool ? tool.id : null;
    els.toolName.value = tool ? tool.name : '';
    els.toolExePath.value = tool ? tool.exePath : '';
    els.toolArgs.value = tool ? tool.args || '' : '';
    els.toolWorkDir.value = tool ? tool.workingDir || '' : '';
    els.toolIsChromium.checked = tool ? Boolean(tool.isChromium) : true;
    els.toolMonitor.checked = tool ? tool.monitor !== false : true;
    els.toolSaveBtn.textContent = tool ? '保存修改' : '添加工具';
    els.toolForm.classList.remove('hidden');
    els.toolName.focus();
  }

  function closeToolForm() {
    if (!els.toolForm) return;
    state.editingToolId = null;
    els.toolForm.classList.add('hidden');
  }

  async function saveToolForm() {
    const exePath = els.toolExePath.value.trim();
    if (!exePath) {
      appendLog('[!] 请先填写或选择工具的可执行文件路径。');
      els.toolExePath.focus();
      return;
    }
    const rawName = els.toolName.value.trim();
    const stem = exePath.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, '') || 'tool';
    const name = rawName || stem;
    const args = els.toolArgs.value.trim();
    const workingDir = els.toolWorkDir.value.trim();
    const isChromium = Boolean(els.toolIsChromium.checked);
    const monitor = Boolean(els.toolMonitor.checked);

    if (state.editingToolId) {
      state.cliTools = state.cliTools.map((t) =>
        t.id === state.editingToolId
          ? { ...t, name, exePath, args, workingDir, isChromium, monitor }
          : t
      );
      appendLog(`[OK] 已更新工具配置: ${name}`);
    } else {
      const baseId = slugifyToolId(name);
      const used = new Set(state.cliTools.map((t) => t.id));
      let nextId = baseId;
      let idx = 2;
      while (used.has(nextId)) {
        nextId = `${baseId}-${idx++}`;
      }
      state.cliTools = [
        ...state.cliTools,
        { id: nextId, name, exePath, args, workingDir, isChromium, monitor },
      ];
      appendLog(`[OK] 已添加外部工具: ${name}`);
    }

    state.cliToolsInitialized = true;
    closeToolForm();
    renderCliTools();
    await persistSettings(true);
    await refreshSnapshot();
  }

  async function detectCliAndTools(overrideCliPath = true) {
    if (!tauriInvoke || state.busy) return;
    setBusy(true);
    try {
      const res = await tauriInvoke('proxy_detect_cli_and_tools');
      let changed = false;

      if (res?.cliPath && (overrideCliPath || !els.inputCliPath.value.trim())) {
        if (normPathKey(els.inputCliPath.value) !== normPathKey(res.cliPath)) {
          els.inputCliPath.value = res.cliPath;
          changed = true;
        }
        appendLog(`[OK] 检测到 Antigravity CLI: ${res.cliPath}`);
        if ((res.cliCandidates || []).length > 1) {
          appendLog(`[i] 共发现 ${res.cliCandidates.length} 个 agy.exe 候选路径。`);
        }
      } else if (!res?.cliPath) {
        appendLog('[!] 未自动发现 agy.exe，请点击「浏览」手动选择。');
      }

      const existingKeys = new Set(state.cliTools.map((t) => normPathKey(t.exePath)));
      const existingIds = new Set(state.cliTools.map((t) => t.id));
      const addedNames = [];
      for (const tool of res?.detectedTools || []) {
        const key = normPathKey(tool.exePath);
        if (!key || existingKeys.has(key) || existingIds.has(tool.id)) continue;
        state.cliTools = [...state.cliTools, tool];
        existingKeys.add(key);
        existingIds.add(tool.id);
        addedNames.push(tool.name);
        changed = true;
      }

      if (addedNames.length) {
        appendLog(`[OK] 已自动添加应用: ${addedNames.join(', ')}`);
      } else if (!overrideCliPath) {
        appendLog('[i] 未发现新的可自动添加应用（Gemini Desktop / T3 Code）。');
      }
      state.cliToolsInitialized = true;
      if (changed) {
        renderCliTools();
        await persistSettings(true);
      }
      await refreshSnapshot();
    } catch (err) {
      appendLog(`[X] 自动探测失败: ${err}`);
    } finally {
      setBusy(false);
    }
  }

  async function runAudit() {
    if (!tauriInvoke) return;
    if (state.auditing) {
      await tauriInvoke('proxy_cancel_audit');
      return;
    }
    if (state.busy) return;
    await persistSettings(true);

    state.auditing = true;
    setBusy(true);
    els.auditBtn.textContent = '取消验收';
    els.auditBar.classList.remove('hidden');
    els.auditFill.style.width = '0%';
    els.auditLabel.textContent = '验收中 0/20s';
    appendLog('[..] 开始 20 秒累计验收检查（请在 Antigravity、agy CLI 或接入工具中触发请求）...');

    try {
      const res = await tauriInvoke('proxy_run_audit', { seconds: 20 });
      if (res.cancelled) {
        appendLog('[i] 已取消本次 20 秒验收检查。');
      } else {
        applySnapshot(res.latestSnapshot, res.cumulativeConnections, {
          proxied: res.cumulativeProxiedCount,
          tun: res.cumulativeTunCount,
          direct: res.cumulativeDirectCount,
        });
        appendLog(res.reportText);
        if (res.reportPath) {
          appendLog(`[i] 验收报告已保存: ${res.reportPath}`);
        }
        setSubView('logs');
      }
    } catch (err) {
      appendLog(`[X] 验收失败: ${err}`);
      setSubView('logs');
    } finally {
      state.auditing = false;
      els.auditBtn.textContent = '验收 20s';
      els.auditBar.classList.add('hidden');
      setBusy(false);
    }
  }

  // Event bindings
  els.launchBtn.addEventListener('click', () => runLaunch(false));
  els.auditBtn.addEventListener('click', () => runAudit());
  els.refreshBtn.addEventListener('click', () => refreshSnapshot());
  els.settingsBtn.addEventListener('click', () => {
    state.settingsExpanded = !state.settingsExpanded;
    els.settingsPanel.classList.toggle('hidden', !state.settingsExpanded);
    els.settingsBtn.textContent = state.settingsExpanded ? '收起设置' : '设置';
  });

  els.confirmOkBtn.addEventListener('click', () => {
    const fn = state.pendingConfirmAction;
    hideConfirm();
    if (fn) fn();
  });
  els.confirmCancelBtn.addEventListener('click', () => {
    hideConfirm();
    appendLog('[i] 已取消操作。');
  });

  els.subViewControl.addEventListener('click', (evt) => {
    const btn = evt.target.closest('.seg-btn');
    if (!btn) return;
    setSubView(btn.dataset.subview);
  });

  els.chkShowIpc.addEventListener('change', () => {
    persistSettings(false);
    renderConnections();
  });

  els.chkAutoPort.addEventListener('change', async () => {
    els.inputPort.disabled = Boolean(els.chkAutoPort.checked);
    await persistSettings(true);
    refreshSnapshot();
  });

  els.chkAutoRefresh.addEventListener('change', () => {
    state.autoRefresh = Boolean(els.chkAutoRefresh.checked);
    if (state.autoRefresh) startAutoRefresh();
    else stopAutoRefresh();
  });

  [
    els.inputHost,
    els.inputPort,
    els.inputAgyPath,
    els.inputCliPath,
    els.inputClashPath,
    els.inputWorkspace,
    els.inputCustomBypass,
  ]
    .filter(Boolean)
    .forEach((input) => input.addEventListener('input', () => persistSettings(false)));
  [els.chkAutoClash, els.chkDisableQuic, els.chkBypassChina]
    .filter(Boolean)
    .forEach((chk) => chk.addEventListener('change', () => persistSettings(false)));
  if (els.btnClearBypass) {
    els.btnClearBypass.addEventListener('click', async () => {
      els.inputCustomBypass.value = '';
      await persistSettings(true);
    });
  }

  async function browseField(kind, inputEl, saveImmediately = true) {
    if (!tauriInvoke) return;
    try {
      const res = await tauriInvoke('proxy_browse_path', { kind, current: inputEl.value });
      if (res?.selected) {
        inputEl.value = res.path;
        if (saveImmediately) {
          await persistSettings(true);
          refreshSnapshot();
        }
      }
    } catch (err) {
      appendLog(`[!] 选择路径失败: ${err}`);
    }
  }

  els.btnBrowseAgy.addEventListener('click', () => browseField('antigravity', els.inputAgyPath));
  if (els.btnBrowseCli) {
    els.btnBrowseCli.addEventListener('click', () => browseField('cli', els.inputCliPath));
  }
  if (els.btnDetectCli) {
    els.btnDetectCli.addEventListener('click', () => detectCliAndTools(true));
  }
  els.btnBrowseClash.addEventListener('click', () => browseField('clash', els.inputClashPath));
  els.btnBrowseWorkspace.addEventListener('click', () =>
    browseField('workspace', els.inputWorkspace)
  );
  els.btnClearWorkspace.addEventListener('click', async () => {
    els.inputWorkspace.value = '';
    await persistSettings(true);
    refreshSnapshot();
  });

  // CLI & Tools event bindings
  if (els.btnCliTerminal) {
    els.btnCliTerminal.addEventListener('click', async () => {
      if (!tauriInvoke || state.busy) return;
      await persistSettings(true);
      setBusy(true);
      try {
        const res = await tauriInvoke('proxy_launch_cli_terminal');
        for (const l of res.logs || []) appendLog(l);
        if (!res.ok && res.error) {
          appendLog(`[X] ${res.error}`);
          setSubView('logs');
        }
      } catch (err) {
        appendLog(`[X] 打开隔离终端失败: ${err}`);
        setSubView('logs');
      } finally {
        setBusy(false);
      }
    });
  }

  if (els.btnTestCli) {
    els.btnTestCli.addEventListener('click', async () => {
      if (!tauriInvoke || state.busy) return;
      await persistSettings(true);
      setBusy(true);
      try {
        const res = await tauriInvoke('proxy_test_cli');
        for (const l of res.logs || []) appendLog(l);
        setSubView('logs');
      } catch (err) {
        appendLog(`[X] CLI 测试异常: ${err}`);
        setSubView('logs');
      } finally {
        setBusy(false);
      }
    });
  }

  async function toggleOrSyncShim(forceEnable) {
    if (!tauriInvoke || state.busy) return;
    await persistSettings(true);
    setBusy(true);
    try {
      const enabled = Boolean(state.snapshot?.cliShim?.enabled);
      const cmd = forceEnable || !enabled ? 'proxy_enable_cli_shim' : 'proxy_disable_cli_shim';
      const res = await tauriInvoke(cmd);
      for (const l of res.logs || []) appendLog(l);
      if (!res.ok && res.error) {
        appendLog(`[X] ${res.error}`);
        setSubView('logs');
      }
      await refreshSnapshot();
    } catch (err) {
      appendLog(`[X] 操作 CLI Shim 失败: ${err}`);
      setSubView('logs');
    } finally {
      setBusy(false);
    }
  }

  if (els.btnToggleShim) {
    els.btnToggleShim.addEventListener('click', () => toggleOrSyncShim(false));
  }
  if (els.btnSyncShim) {
    els.btnSyncShim.addEventListener('click', () => toggleOrSyncShim(true));
  }

  if (els.btnCopySnippets) {
    els.btnCopySnippets.addEventListener('click', async () => {
      if (!tauriInvoke) return;
      await persistSettings(true);
      try {
        const snippets = await tauriInvoke('proxy_cli_snippets');
        appendLog(`[终端代理变量 (${snippets.proxyUrl})]`);
        appendLog(`  PowerShell : ${snippets.powershell}`);
        appendLog(`  CMD        : ${snippets.cmd}`);
        appendLog(`  Bash / Zsh : ${snippets.bash}`);
        try {
          await navigator.clipboard.writeText(snippets.powershell);
          const orig = els.btnCopySnippets.textContent;
          els.btnCopySnippets.textContent = '已复制 PS';
          setTimeout(() => {
            els.btnCopySnippets.textContent = orig;
          }, 1400);
          appendLog('[OK] 已将 PowerShell 隔离代理命令复制到剪贴板（CMD/Bash 命令见上方日志）。');
        } catch {
          appendLog('[!] 无法写入剪贴板，请从日志中手动复制。');
          setSubView('logs');
        }
      } catch (err) {
        appendLog(`[X] 生成终端命令失败: ${err}`);
      }
    });
  }

  if (els.btnStopCli) {
    els.btnStopCli.addEventListener('click', async () => {
      if (!tauriInvoke || state.busy) return;
      const cliProcs = state.snapshot?.cliProcCount || 0;
      if (cliProcs === 0) {
        appendLog('[i] 当前没有正在运行的 agy CLI 进程。');
        await refreshSnapshot();
        return;
      }
      showConfirm(`确定要结束当前正在运行的 ${cliProcs} 个 agy CLI 进程吗？`, async () => {
        setBusy(true);
        try {
          const res = await tauriInvoke('proxy_stop_cli_processes');
          for (const l of res.logs || []) appendLog(l);
          await refreshSnapshot();
        } catch (err) {
          appendLog(`[X] 结束 CLI 进程失败: ${err}`);
        } finally {
          setBusy(false);
        }
      });
    });
  }

  if (els.btnDetectTools) {
    els.btnDetectTools.addEventListener('click', () => detectCliAndTools(false));
  }

  if (els.btnAddTool) {
    els.btnAddTool.addEventListener('click', () => {
      if (els.toolForm && !els.toolForm.classList.contains('hidden') && !state.editingToolId) {
        closeToolForm();
      } else {
        openToolForm(null);
      }
    });
  }
  if (els.toolBrowseExe) {
    els.toolBrowseExe.addEventListener('click', () =>
      browseField('tool', els.toolExePath, false)
    );
  }
  if (els.toolBrowseDir) {
    els.toolBrowseDir.addEventListener('click', () =>
      browseField('tool_dir', els.toolWorkDir, false)
    );
  }
  if (els.toolSaveBtn) {
    els.toolSaveBtn.addEventListener('click', () => saveToolForm());
  }
  if (els.toolCancelBtn) {
    els.toolCancelBtn.addEventListener('click', () => closeToolForm());
  }

  if (els.cliToolsBody) {
    els.cliToolsBody.addEventListener('click', async (evt) => {
      const detectBtn = evt.target.closest('#proxyBtnAutoDetectTools');
      if (detectBtn) {
        detectCliAndTools(false);
        return;
      }
      const launchBtn = evt.target.closest('.cli-tool-launch-btn');
      if (launchBtn) {
        runLaunchTool(launchBtn.dataset.toolId, false);
        return;
      }
      const stopBtn = evt.target.closest('.cli-tool-stop-btn');
      if (stopBtn) {
        runStopTool(stopBtn.dataset.toolId);
        return;
      }
      const editBtn = evt.target.closest('.cli-tool-edit-btn');
      if (editBtn) {
        const tool = state.cliTools.find((t) => t.id === editBtn.dataset.toolId);
        if (tool) openToolForm(tool);
        return;
      }
      const delBtn = evt.target.closest('.cli-tool-del-btn');
      if (delBtn) {
        const id = delBtn.dataset.toolId;
        const tool = state.cliTools.find((t) => t.id === id);
        state.cliTools = state.cliTools.filter((t) => t.id !== id);
        state.cliToolsInitialized = true;
        if (state.editingToolId === id) closeToolForm();
        renderCliTools();
        await persistSettings(true);
        if (tool) appendLog(`[i] 已移除工具配置: ${tool.name}`);
        await refreshSnapshot();
      }
    });
  }

  els.btnStartClash.addEventListener('click', async () => {
    if (!tauriInvoke || state.busy) return;
    await persistSettings(true);
    setBusy(true);
    try {
      const res = await tauriInvoke('proxy_start_clash');
      for (const l of res.logs || []) appendLog(l);
      await refreshSnapshot();
    } catch (err) {
      appendLog(`[X] 启动 Clash 失败: ${err}`);
    } finally {
      setBusy(false);
    }
  });

  els.btnFlushDns.addEventListener('click', async () => {
    if (!tauriInvoke || state.busy) return;
    setBusy(true);
    try {
      const res = await tauriInvoke('proxy_flush_dns');
      for (const l of res.logs || []) appendLog(l);
      setSubView('logs');
    } catch (err) {
      appendLog(`[X] 刷新 DNS 失败: ${err}`);
    } finally {
      setBusy(false);
    }
  });

  els.btnDryRun.addEventListener('click', async () => {
    if (!tauriInvoke) return;
    await persistSettings(true);
    try {
      const text = await tauriInvoke('proxy_dry_run');
      appendLog(text);
      setSubView('logs');
    } catch (err) {
      appendLog(`[X] 生成预览失败: ${err}`);
    }
  });

  els.btnStopAgy.addEventListener('click', async () => {
    if (!tauriInvoke || state.busy) return;
    const totalProcs =
      (state.snapshot?.antigravityProcCount || 0) + (state.snapshot?.languageServerProcCount || 0);
    if (totalProcs === 0) {
      appendLog('[i] 当前没有正在运行的 Antigravity 进程。');
      await refreshSnapshot();
      return;
    }
    showConfirm(`确定要结束当前正在运行的 ${totalProcs} 个 Antigravity / language_server 进程吗？`, async () => {
      setBusy(true);
      try {
        const res = await tauriInvoke('proxy_stop_antigravity');
        for (const l of res.logs || []) appendLog(l);
        await refreshSnapshot();
      } catch (err) {
        appendLog(`[X] 结束进程失败: ${err}`);
      } finally {
        setBusy(false);
      }
    });
  });

  els.btnOpenReport.addEventListener('click', async () => {
    if (!tauriInvoke) return;
    try {
      await tauriInvoke('proxy_open_report');
    } catch (err) {
      appendLog(`[!] ${err}`);
    }
  });

  els.btnCopyLog.addEventListener('click', async () => {
    if (!state.logs.length) return;
    try {
      await navigator.clipboard.writeText(state.logs.join('\n'));
      const orig = els.btnCopyLog.textContent;
      els.btnCopyLog.textContent = '已复制';
      setTimeout(() => {
        els.btnCopyLog.textContent = orig;
      }, 1200);
    } catch {
      appendLog('[!] 无法写入剪贴板。');
    }
  });

  els.btnClearLog.addEventListener('click', () => {
    state.logs = [];
    els.logPre.textContent = '';
  });

  let initialized = false;
  async function ensureLoaded() {
    if (initialized) return;
    initialized = true;
    appendLog('[i] 就绪。点击「启动 Antigravity」即可隔离走本地代理（不改系统代理、不开 TUN）。');

    if (tauriListen) {
      state.unlistenAudit = await tauriListen('proxy-audit-progress', (event) => {
        const p = event.payload;
        if (!p) return;
        const pct = Math.min(100, Math.round((p.elapsed / Math.max(1, p.total)) * 100));
        els.auditFill.style.width = `${pct}%`;
        els.auditLabel.textContent = `验收中 ${p.elapsed}/${p.total}s`;
        if (p.snapshot) applySnapshot(p.snapshot);
      });
    }

    if (tauriInvoke) {
      try {
        const s = await tauriInvoke('proxy_get_settings');
        applySettingsToForm(s);
      } catch (err) {
        appendLog(`[!] 加载设置失败: ${err}`);
      }
      await refreshSnapshot();
    }
  }

  return {
    setActive(active) {
      state.active = active;
      if (active) {
        ensureLoaded().then(() => {
          refreshSnapshot();
          startAutoRefresh();
        });
      } else {
        stopAutoRefresh();
      }
    },
  };
}
