import {
  formatCreditsSummary,
  formatRemainingPct,
  formatResetCountdown,
  formatResetExact,
  normalizeEmailKey,
  quotaToneClass,
  renderQuotaDetailHtml,
  tierBadgeHtml,
} from './quota.js';

const tauriInvoke =
  typeof window !== 'undefined' && typeof window.__TAURI__?.core?.invoke === 'function'
    ? window.__TAURI__.core.invoke
    : null;

const tauriListen =
  typeof window !== 'undefined' && typeof window.__TAURI__?.event?.listen === 'function'
    ? window.__TAURI__.event.listen
    : null;

const WAIT_TIMEOUT_MS = 300_000;
const ACTIVE_REFRESH_MS = 5_000;
const LOGIN_POLL_MS = 1_200;
const QUOTA_STALE_MS = 180_000;
const QUOTA_TICK_MS = 30_000;
const RESTART_PREF_KEY = 'agydock_accounts_restart';

function escapeHtml(str) {
  return String(str ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function pad2(n) {
  return String(n).padStart(2, '0');
}

function nowTimeStr() {
  const d = new Date();
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

function formatDateTime(ms) {
  if (!ms) return '—';
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(
    d.getMinutes()
  )}`;
}

function formatShortDate(ms) {
  if (!ms) return '—';
  const d = new Date(ms);
  return `${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function formatRelativeTime(ms) {
  if (!ms) return '—';
  const diffSec = Math.floor((Date.now() - ms) / 1000);
  if (diffSec < 60) return '刚刚';
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)} 分钟前`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} 小时前`;
  const days = Math.floor(diffSec / 86400);
  if (days < 30) return `${days} 天前`;
  return formatShortDate(ms);
}

function formatExpiryDelta(ms) {
  if (!ms) return '到期时间未知';
  const diffMs = ms - Date.now();
  const absMin = Math.round(Math.abs(diffMs) / 60_000);
  const label =
    absMin < 1
      ? '<1m'
      : absMin < 60
      ? `${absMin}m`
      : absMin < 24 * 60
      ? `${Math.floor(absMin / 60)}h${absMin % 60 ? ` ${absMin % 60}m` : ''}`
      : `${Math.floor(absMin / (24 * 60))}d`;
  return diffMs > 0 ? `剩余 ${label}` : `已过期 ${label}`;
}

function loadRestartPref() {
  try {
    const raw = window.localStorage.getItem(RESTART_PREF_KEY);
    return raw === null ? true : raw === '1';
  } catch {
    return true;
  }
}

function saveRestartPref(val) {
  try {
    window.localStorage.setItem(RESTART_PREF_KEY, val ? '1' : '0');
  } catch {
    // ignore storage failures
  }
}

export function initAccountsModule() {
  if (!tauriInvoke) {
    return { setActive() {} };
  }

  const els = {
    tabBtn: document.getElementById('accountsTabBtn'),
    view: document.getElementById('accountsView'),
    controls: document.getElementById('accountsControls'),
    addBtn: document.getElementById('accountAddBtn'),
    saveBtn: document.getElementById('accountSaveBtn'),
    logoutBtn: document.getElementById('accountLogoutBtn'),
    quotaAllBtn: document.getElementById('accountQuotaAllBtn'),
    refreshBtn: document.getElementById('accountRefreshBtn'),
    detectSaveBtn: document.getElementById('accountDetectSaveBtn'),

    notice: document.getElementById('accountNotice'),
    noticeText: document.getElementById('accountNoticeText'),
    noticeCloseBtn: document.getElementById('accountNoticeCloseBtn'),
    confirmBanner: document.getElementById('accountConfirmBanner'),
    confirmText: document.getElementById('accountConfirmText'),
    confirmOkBtn: document.getElementById('accountConfirmOkBtn'),
    confirmCancelBtn: document.getElementById('accountConfirmCancelBtn'),

    waitBar: document.getElementById('accountWaitBar'),
    waitLabel: document.getElementById('accountWaitLabel'),
    waitFill: document.getElementById('accountWaitFill'),
    waitCancelBtn: document.getElementById('accountWaitCancelBtn'),
    waitStopOnlyBtn: document.getElementById('accountWaitStopOnlyBtn'),

    currentDot: document.getElementById('accCurrentDot'),
    currentEmail: document.getElementById('accCurrentEmail'),
    currentTierBadge: document.getElementById('accCurrentTierBadge'),
    currentSub: document.getElementById('accCurrentSub'),
    count: document.getElementById('accCount'),
    countSub: document.getElementById('accCountSub'),
    envDot: document.getElementById('accEnvDot'),
    envValue: document.getElementById('accEnvValue'),
    envSub: document.getElementById('accEnvSub'),
    lastUsed: document.getElementById('accLastUsed'),
    lastUsedSub: document.getElementById('accLastUsedSub'),

    listNote: document.getElementById('accListNote'),
    restartChk: document.getElementById('accountRestartChk'),
    searchInput: document.getElementById('accountSearch'),
    sortSelect: document.getElementById('accountSort'),
    verifyBtn: document.getElementById('accountVerifyBtn'),
    openDirBtn: document.getElementById('accountOpenDirBtn'),
    oauthBtn: document.getElementById('accountOAuthBtn'),
    oauthPanel: document.getElementById('accountOAuthPanel'),
    oauthStatusTag: document.getElementById('accountOAuthStatusTag'),
    oauthCloseBtn: document.getElementById('accountOAuthCloseBtn'),
    oauthClientId: document.getElementById('accountOAuthClientId'),
    oauthClientSecret: document.getElementById('accountOAuthClientSecret'),
    oauthSaveBtn: document.getElementById('accountOAuthSaveBtn'),
    oauthClearBtn: document.getElementById('accountOAuthClearBtn'),
    oauthPathNote: document.getElementById('accountOAuthPathNote'),
    tableBody: document.getElementById('accountsTableBody'),

    logCount: document.getElementById('accountLogCount'),
    logPreview: document.getElementById('accountLogPreview'),
    logToggleBtn: document.getElementById('accountLogToggleBtn'),
    logBody: document.getElementById('accountLogBody'),
    logPre: document.getElementById('accountLogPre'),
    logCopyBtn: document.getElementById('accountLogCopyBtn'),
    logClearBtn: document.getElementById('accountLogClearBtn'),

    footerLeft: document.getElementById('accountFooterLeft'),
    footerRight: document.getElementById('accountFooterRight'),
  };

  const state = {
    active: false,
    initialized: false,
    loading: false,
    busy: false,
    data: null,
    runtime: null,
    healthMap: new Map(),
    logs: [],
    logsExpanded: false,
    waiting: false,
    pollInFlight: false,
    waitTimer: null,
    pollTimer: null,
    waitDeadline: 0,
    previousEmail: null,
    renamingId: null,
    pendingConfirm: null,
    activeTimer: null,
    countdownTimer: null,
    restartOnSwitch: loadRestartPref(),
    search: '',
    sort: 'default',
    quotaMap: new Map(),
    quotaLoadingIds: new Set(),
    quotaFetchingAll: false,
    expandedQuotaId: null,
    unlistenTray: null,
  };

  if (els.restartChk) {
    els.restartChk.checked = state.restartOnSwitch;
  }

  function head(value) {
    return String(value ?? '').trim().toLowerCase();
  }

  const isCurrent = (email) =>
    Boolean(email) && head(state.data?.current?.email) === head(email);

  function setLogsExpanded(expanded) {
    state.logsExpanded = expanded;
    els.logBody.classList.toggle('hidden', !expanded);
    els.logToggleBtn.textContent = expanded ? '收起日志' : '展开日志';
    if (expanded) {
      els.logPre.scrollTop = els.logPre.scrollHeight;
    }
  }

  function appendLog(message) {
    let lastLine = '';
    for (const line of String(message ?? '').split(/\r?\n/)) {
      if (!line.trim()) continue;
      const formatted = `[${nowTimeStr()}] ${line}`;
      state.logs.push(formatted);
      lastLine = formatted;
    }
    if (state.logs.length > 400) state.logs = state.logs.slice(-400);
    els.logPre.textContent = state.logs.join('\n');
    els.logPre.scrollTop = els.logPre.scrollHeight;
    els.logCount.textContent = String(state.logs.length);
    if (lastLine) {
      els.logPreview.textContent = lastLine;
      els.logPreview.title = lastLine;
    }
  }

  function showNotice(text, kind = 'ok') {
    if (!text) {
      els.notice.className = 'notice hidden';
      els.noticeText.textContent = '';
      return;
    }
    const cls =
      kind === 'error'
        ? 'notice notice-error'
        : kind === 'ok'
        ? 'notice notice-ok'
        : 'notice';
    els.notice.className = cls;
    els.noticeText.textContent = text;
  }

  function showConfirm(message, onConfirm) {
    state.pendingConfirm = onConfirm;
    els.confirmText.textContent = message;
    els.confirmBanner.classList.remove('hidden');
    els.confirmBanner.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  }

  function hideConfirm() {
    state.pendingConfirm = null;
    els.confirmBanner.classList.add('hidden');
  }

  function updateTabBadge() {
    if (!els.tabBtn) return;
    els.tabBtn.textContent = state.waiting ? '账号管理 · 等待登录' : '账号管理';
  }

  function setBusy(busy) {
    state.busy = busy;
    const locked = busy || state.waiting;
    const present = Boolean(state.data?.current?.present);

    if (els.addBtn) els.addBtn.disabled = locked;
    if (els.saveBtn) els.saveBtn.disabled = locked;
    if (els.logoutBtn) els.logoutBtn.disabled = locked || !present;
    if (els.quotaAllBtn) els.quotaAllBtn.disabled = locked || state.quotaFetchingAll;
    if (els.verifyBtn) els.verifyBtn.disabled = locked;
    if (els.refreshBtn) els.refreshBtn.disabled = busy;

    els.tableBody.querySelectorAll('button[data-act]').forEach((btn) => {
      const act = btn.dataset.act;
      if (btn.dataset.locked === '1') {
        btn.disabled = true;
      } else if (act === 'quota' || act === 'quota-refresh') {
        btn.disabled =
          locked || state.quotaFetchingAll || state.quotaLoadingIds.has(btn.dataset.id);
      } else {
        btn.disabled = locked;
      }
    });
  }

  async function copyText(text, label) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      showNotice(`已复制${label}: ${text}`, 'ok');
    } catch {
      appendLog(`[!] 无法写入剪贴板: ${text}`);
    }
  }

  function renderKpis(data) {
    const current = data.current || {};
    const present = Boolean(current.present);
    const email = current.email || null;
    const accounts = data.accounts || [];

    // KPI 1: 当前登录账号
    els.currentEmail.textContent = email || '未登录';
    els.currentEmail.title = email ? `${email}（点击复制）` : '';
    els.currentEmail.classList.toggle('acc-copyable', Boolean(email));
    els.currentDot.className = 'status-dot';
    const currentEntry = accounts.find((a) => email && a.email === head(email));
    const currentQuota = email ? state.quotaMap.get(normalizeEmailKey(email)) : null;
    if (els.currentTierBadge) {
      els.currentTierBadge.innerHTML =
        present && currentQuota?.plan ? tierBadgeHtml(currentQuota.plan) : '';
    }
    const remarkPrefix = currentEntry?.name ? `${currentEntry.name} · ` : '';
    const quotaSuffix =
      currentQuota && !currentQuota.error && currentQuota.minRemainingFraction != null
        ? ` · 最低剩余 ${formatRemainingPct(currentQuota.minRemainingFraction)} (${formatResetCountdown(
            currentQuota.earliestResetAtMs
          )})`
        : '';
    if (!present) {
      els.currentDot.classList.add('muted');
      els.currentSub.textContent = '请在 Antigravity 中登录，或点「添加账号」';
      els.currentSub.title = '';
    } else if (current.hasRefreshToken) {
      els.currentSub.textContent = `${remarkPrefix}登录正常 · 含刷新令牌（支持自动续期）${quotaSuffix}`;
      els.currentSub.title = current.expiresAtMs
        ? `最近签发 access_token 状态: ${formatExpiryDelta(current.expiresAtMs)}（${formatDateTime(
            current.expiresAtMs
          )}），Antigravity 运行时会按需用 refresh_token 自动续期`
        : '';
    } else if (current.expired) {
      els.currentDot.classList.add('err');
      els.currentSub.textContent = `${remarkPrefix}凭据已失效 · 缺少 refresh_token，需重新登录${quotaSuffix}`;
      els.currentSub.title = '';
    } else {
      els.currentDot.classList.add('warn');
      els.currentSub.textContent = `${remarkPrefix}临时凭据 (${formatExpiryDelta(
        current.expiresAtMs
      )}) · 无刷新令牌${quotaSuffix}`;
      els.currentSub.title = '';
    }

    // KPI 2: 已保存账号与备份健康
    const total = accounts.length;
    const brokenCount = accounts.filter((a) => {
      const h = state.healthMap.get(a.id);
      if (h) return h.status !== 'ok';
      return a.backupOk === false;
    }).length;
    const noRefreshCount = accounts.filter((a) => !a.hasRefreshToken).length;

    els.count.textContent = String(total);
    if (!total) {
      els.countSub.textContent = '登录过的账号会自动归档到这里';
    } else if (brokenCount > 0) {
      els.countSub.textContent = `${brokenCount} 个本地备份异常 · 建议检查或重新保存`;
    } else if (noRefreshCount > 0) {
      els.countSub.textContent = `DPAPI 加密 · ${noRefreshCount} 个缺少 refresh_token`;
    } else {
      els.countSub.textContent = 'DPAPI 加密备份 · 全部可免登录切换';
    }

    // KPI 3: 运行与代理环境
    const rt = state.runtime;
    els.envDot.className = 'status-dot';
    if (!rt) {
      els.envDot.classList.add('muted');
      els.envValue.textContent = '检测中...';
      els.envSub.textContent = '正在读取 Antigravity 与隔离代理状态';
    } else if (!rt.exeValid) {
      els.envDot.classList.add('err');
      els.envValue.textContent = '路径无效';
      els.envSub.textContent = '请在「隔离代理 → 设置」中指定 Antigravity.exe';
    } else {
      const procText =
        rt.runningCount > 0 ? `运行中 (${rt.runningCount} 进程)` : '未运行';
      els.envValue.textContent = procText;
      if (rt.proxyListening) {
        els.envSub.textContent = `隔离代理就绪 · ${rt.proxyEndpoint}`;
      } else if (rt.autoStartClash) {
        els.envDot.classList.add('warn');
        els.envSub.textContent = `代理未监听 · 重启时将尝试拉起 Clash`;
      } else {
        els.envDot.classList.add('warn');
        els.envSub.textContent = `代理未监听 · 重启时将以直连方式启动`;
      }
    }

    // KPI 4: 最近切换（全账号维度，修复此前仅查当前账号导致空白的问题）
    let latestSwitched = null;
    let totalSwitches = 0;
    for (const acc of accounts) {
      totalSwitches += acc.switchCount || 0;
      if (acc.lastUsedAtMs && (!latestSwitched || acc.lastUsedAtMs > latestSwitched.lastUsedAtMs)) {
        latestSwitched = acc;
      }
    }
    if (latestSwitched) {
      els.lastUsed.textContent = formatRelativeTime(latestSwitched.lastUsedAtMs);
      els.lastUsed.title = formatDateTime(latestSwitched.lastUsedAtMs);
      const who = latestSwitched.name || latestSwitched.email;
      els.lastUsedSub.textContent = `${who} · 累计切换 ${totalSwitches} 次`;
    } else {
      els.lastUsed.textContent = '—';
      els.lastUsed.title = '';
      els.lastUsedSub.textContent = '尚未通过本工具切换过账号';
    }

    els.footerLeft.textContent = `账号目录: ${data.storeDir || '—'} · 托盘右键可快速切换`;
    els.footerRight.textContent = `更新于 ${nowTimeStr()}`;

    const detectedUnsaved =
      present && email && !accounts.some((a) => a.email === head(email));
    els.detectSaveBtn.classList.toggle('hidden', !detectedUnsaved);
    if (detectedUnsaved) {
      els.detectSaveBtn.textContent = data.ignoredCurrent
        ? `重新归档 ${email}`
        : `归档当前账号 (${email})`;
    }
  }

  function getFilteredSortedAccounts(accounts) {
    const q = state.search.trim().toLowerCase();
    let list = accounts;
    if (q) {
      list = accounts.filter(
        (acc) =>
          acc.email.toLowerCase().includes(q) ||
          (acc.name && acc.name.toLowerCase().includes(q)) ||
          (acc.authMethod && acc.authMethod.toLowerCase().includes(q))
      );
    }

    if (state.sort === 'default') {
      return list;
    }

    const sorted = [...list];
    if (state.sort === 'quota') {
      sorted.sort((a, b) => {
        const qa = state.quotaMap.get(normalizeEmailKey(a.email));
        const qb = state.quotaMap.get(normalizeEmailKey(b.email));
        const fa = qa?.minRemainingFraction ?? -1;
        const fb = qb?.minRemainingFraction ?? -1;
        return fb - fa || a.email.localeCompare(b.email);
      });
    } else if (state.sort === 'recent') {
      sorted.sort((a, b) => (b.lastUsedAtMs || 0) - (a.lastUsedAtMs || 0));
    } else if (state.sort === 'switches') {
      sorted.sort(
        (a, b) =>
          (b.switchCount || 0) - (a.switchCount || 0) ||
          (b.lastUsedAtMs || 0) - (a.lastUsedAtMs || 0)
      );
    } else if (state.sort === 'saved') {
      sorted.sort((a, b) => (b.savedAtMs || 0) - (a.savedAtMs || 0));
    } else if (state.sort === 'email') {
      sorted.sort((a, b) => a.email.localeCompare(b.email));
    }
    return sorted;
  }

  function renderQuotaCellHtml(acc) {
    const isLoading = state.quotaLoadingIds.has(acc.id) || (state.quotaFetchingAll && acc.backupOk !== false);
    const snap = state.quotaMap.get(normalizeEmailKey(acc.email));

    if (isLoading && !snap) {
      return '<div class="quota-cell-empty mono">查询中...</div>';
    }
    if (!snap) {
      return '<div class="quota-cell-empty mono">未查询 · 点击获取</div>';
    }

    const plan = snap.plan || {};
    const creditsText = formatCreditsSummary(plan);
    const loadingTag = isLoading ? '<span class="tag">查询中</span>' : '';
    const errTag = snap.error
      ? `<span class="tag tag-err" title="${escapeHtml(snap.error)}">查询异常</span>`
      : '';
    const topLine = `
      <div class="quota-cell-top">
        ${tierBadgeHtml(plan)}
        <span class="quota-cell-plan" title="${escapeHtml(plan.tierLabel || '')}">${escapeHtml(
          plan.tierLabel || ''
        )}</span>
        ${creditsText ? `<span class="tag">${escapeHtml(creditsText)}</span>` : ''}
        ${loadingTag}
        ${errTag}
      </div>
    `;

    const pools = Array.isArray(snap.pools) ? snap.pools : [];
    if (pools.length > 0) {
      const minFrac = snap.minRemainingFraction;
      const pctText = formatRemainingPct(minFrac);
      const tone = quotaToneClass(minFrac);
      const width =
        minFrac === null || minFrac === undefined
          ? 0
          : Math.round(Math.max(0, Math.min(1, Number(minFrac))) * 100);
      const resetText = formatResetCountdown(snap.earliestResetAtMs);
      const resetTitle = snap.earliestResetAtMs
        ? `最近刷新: ${formatResetExact(snap.earliestResetAtMs)}`
        : '';
      const poolSummary = pools
        .slice(0, 3)
        .map((p) => {
          const shortName = p.label
            .replace('Gemini ', '')
            .replace(' 模型', '')
            .replace('Claude / GPT', '3P');
          return `${shortName} ${formatRemainingPct(p.remainingFraction)}`;
        })
        .join(' · ');

      return `
        ${topLine}
        <div class="quota-mini-row" title="${escapeHtml(resetTitle)}">
          <div class="quota-bar-track"><div class="quota-bar-fill tone-${tone}" style="width:${width}%"></div></div>
          <span class="quota-mini-pct mono tone-${tone}">${pctText}</span>
          <span class="quota-mini-reset mono">${escapeHtml(resetText)}</span>
        </div>
        <div class="quota-cell-sub mono" title="${escapeHtml(
          pools.map((p) => `${p.label}: ${formatRemainingPct(p.remainingFraction)}`).join(' · ')
        )}">${escapeHtml(poolSummary)} · 点击查看明细</div>
      `;
    }

    if (snap.error) {
      return `
        ${topLine}
        <div class="quota-cell-sub tone-err" title="${escapeHtml(snap.error)}">${escapeHtml(
          snap.error
        )}</div>
      `;
    }

    return `
      ${topLine}
      <div class="quota-cell-sub mono">暂无额度池 · 点击展开详情</div>
    `;
  }

  function renderTable(data) {
    const allAccounts = data.accounts || [];
    const accounts = getFilteredSortedAccounts(allAccounts);

    if (!allAccounts.length) {
      els.tableBody.innerHTML = `<tr class="empty-row"><td colspan="6">${
        data.current?.present
          ? '当前登录账号尚未归档（可能曾被移除），请点击上方「重新归档」或「保存当前账号」'
          : '还没有保存的账号 · 点击右上角「添加账号」登录一次即可永久免登录切换'
      }</td></tr>`;
      els.listNote.textContent = '登录过一次的账号会保存在本地，之后可直接切换';
      return;
    }

    if (!accounts.length) {
      els.tableBody.innerHTML =
        '<tr class="empty-row"><td colspan="6">没有匹配筛选条件的账号</td></tr>';
      els.listNote.textContent = `筛选匹配 0 / ${allAccounts.length} 个账号`;
      return;
    }

    const switchLabel = state.restartOnSwitch ? '切换并重启' : '切换凭据';

    els.tableBody.innerHTML = accounts
      .map((acc) => {
        const current = isCurrent(acc.email);
        const expired = acc.expiresAtMs ? acc.expiresAtMs <= Date.now() : false;
        const health = state.healthMap.get(acc.id);
        const backupBroken = health ? health.status !== 'ok' : acc.backupOk === false;

        const stateTags = [];
        if (current) stateTags.push('<span class="tag tag-ok">当前</span>');
        if (backupBroken) {
          const reason =
            health?.status === 'corrupted'
              ? '备份损坏'
              : '备份缺失';
          stateTags.push(
            `<span class="tag tag-err" title="${escapeHtml(
              health?.detail || '本地加密凭据文件缺失或不可解密'
            )}">${reason}</span>`
          );
        } else if (!acc.hasRefreshToken) {
          if (expired) {
            stateTags.push(
              '<span class="tag tag-err" title="缺少 refresh_token 且 access_token 已过期，需重新登录">凭据失效</span>'
            );
          } else {
            stateTags.push(
              '<span class="tag tag-warn" title="缺少 refresh_token，access_token 过期后可能需要重新登录">临时凭据</span>'
            );
          }
        } else {
          const tokenHint = acc.expiresAtMs
            ? `含长期 refresh_token，可免登录切换并按需自动续期（缓存 access_token: ${formatExpiryDelta(
                acc.expiresAtMs
              )}）`
            : '含长期 refresh_token，可免登录切换并自动续期';
          stateTags.push(
            `<span class="tag ${current ? '' : 'tag-ok'}" title="${escapeHtml(
              tokenHint
            )}">长期有效</span>`
          );
        }
        if (health?.status === 'ok') {
          stateTags.push('<span class="tag" title="已通过 DPAPI 解密与结构校验">已校验</span>');
        }

        const metaParts = [];
        if (acc.authMethod) metaParts.push(acc.authMethod);
        if (acc.hasRefreshToken) {
          metaParts.push('含刷新令牌');
        } else if (acc.expiresAtMs) {
          metaParts.push(`临时令牌 ${formatExpiryDelta(acc.expiresAtMs)}`);
        }
        metaParts.push(`保存于 ${formatShortDate(acc.savedAtMs)}`);
        const metaTooltip =
          acc.hasRefreshToken && acc.expiresAtMs
            ? `缓存 access_token 签发有效期: ${formatDateTime(acc.expiresAtMs)}（${formatExpiryDelta(
                acc.expiresAtMs
              )}，运行时由 refresh_token 自动续期）`
            : '';

        const pinBadge = acc.pinned ? '<span class="tag tag-pin">置顶</span>' : '';

        const renameCell =
          state.renamingId === acc.id
            ? `<input class="text-input acc-rename-input mono" data-rename-input="${escapeHtml(
                acc.id
              )}" value="${escapeHtml(acc.name || '')}" maxlength="48" placeholder="输入备注，回车保存" />`
            : `<span class="acc-name" data-act="rename" data-id="${escapeHtml(
                acc.id
              )}" title="点击编辑备注">${
                acc.name ? escapeHtml(acc.name) : '<span class="acc-name-empty">点击添加备注</span>'
              }</span>`;

        const usagePrimary = acc.lastUsedAtMs
          ? formatRelativeTime(acc.lastUsedAtMs)
          : current
          ? '当前会话'
          : '尚未切换';
        const usageSub =
          acc.switchCount > 0
            ? `已切换 ${acc.switchCount} 次`
            : `归档 ${formatShortDate(acc.savedAtMs)}`;
        const usageTitle = acc.lastUsedAtMs
          ? `最近切换于 ${formatDateTime(acc.lastUsedAtMs)}`
          : `归档于 ${formatDateTime(acc.savedAtMs)}`;

        const switchLocked = current || backupBroken;
        const switchBtnText = current
          ? '当前账号'
          : backupBroken
          ? '备份异常'
          : switchLabel;
        const switchTitle = current
          ? '已是当前登录账号'
          : backupBroken
          ? '本地备份文件缺失或损坏，无法切换'
          : state.restartOnSwitch
          ? '关闭 Antigravity，写入该账号凭据并重新启动'
          : '仅将该账号凭据写入 Windows 凭据管理器（不重启 Antigravity）';

        const isQuotaLoading =
          state.quotaLoadingIds.has(acc.id) || (state.quotaFetchingAll && acc.backupOk !== false);
        const isExpanded = state.expandedQuotaId === acc.id;
        const quotaSnap = state.quotaMap.get(normalizeEmailKey(acc.email));

        const detailRowHtml = isExpanded
          ? `<tr class="detail-row"><td colspan="6">${renderQuotaDetailHtml(quotaSnap, {
              showRefreshBtn: true,
              accountId: acc.id,
            })}</td></tr>`
          : '';

        return `
          <tr class="${current || isExpanded ? 'selected' : ''}">
            <td>
              <div class="acc-email-row">
                <span class="acc-email mono acc-copyable" data-copy-email="${escapeHtml(
                  acc.email
                )}" title="点击复制邮箱: ${escapeHtml(acc.email)}">${escapeHtml(acc.email)}</span>
                ${pinBadge}
              </div>
              <div class="acc-meta mono" title="${escapeHtml(metaTooltip)}">${escapeHtml(
                metaParts.join(' · ')
              )}</div>
            </td>
            <td class="quota-cell" data-act="toggle-quota" data-id="${escapeHtml(acc.id)}" title="点击展开或查询额度明细">
              ${renderQuotaCellHtml(acc)}
            </td>
            <td>${renameCell}</td>
            <td>
              <div class="acc-usage-primary mono" title="${escapeHtml(
                usageTitle
              )}">${escapeHtml(usagePrimary)}</div>
              <div class="acc-usage-sub mono">${escapeHtml(usageSub)}</div>
            </td>
            <td><div class="acc-tags">${stateTags.join('')}</div></td>
            <td>
              <div class="acc-actions">
                <button type="button" class="btn btn-primary" data-act="switch" data-id="${escapeHtml(
                  acc.id
                )}" data-locked="${switchLocked ? '1' : '0'}" ${
                  switchLocked ? 'disabled' : ''
                } title="${escapeHtml(switchTitle)}">${switchBtnText}</button>
                <button type="button" class="btn" data-act="quota" data-id="${escapeHtml(
                  acc.id
                )}" ${isQuotaLoading ? 'disabled' : ''} title="查询该账号套餐等级与剩余额度">${
                  isQuotaLoading ? '查询中' : '额度'
                }</button>
                <button type="button" class="btn acc-pin-btn${
                  acc.pinned ? ' pinned' : ''
                }" data-act="pin" data-id="${escapeHtml(acc.id)}" title="${
                  acc.pinned ? '取消置顶' : '置顶该账号'
                }">${acc.pinned ? '已置顶' : '置顶'}</button>
                <button type="button" class="btn" data-act="rename-btn" data-id="${escapeHtml(
                  acc.id
                )}">备注</button>
                <button type="button" class="btn btn-danger-subtle" data-act="remove" data-id="${escapeHtml(
                  acc.id
                )}">删除</button>
              </div>
            </td>
          </tr>
          ${detailRowHtml}
        `;
      })
      .join('');

    els.listNote.textContent =
      accounts.length === allAccounts.length
        ? `共 ${allAccounts.length} 个账号 · 登录过一次即可免登录切换`
        : `筛选显示 ${accounts.length} / ${allAccounts.length} 个账号`;

    if (state.renamingId) {
      const input = els.tableBody.querySelector('[data-rename-input]');
      if (input) {
        input.focus();
        input.select();
      }
    }
  }

  function render() {
    if (!state.data) return;
    renderKpis(state.data);
    renderTable(state.data);
    attachRowHandlers();
    setBusy(state.busy);
  }

  async function refreshList({ silent = false } = {}) {
    if (state.loading) return;
    state.loading = true;
    if (!silent && els.refreshBtn) {
      els.refreshBtn.textContent = '刷新中...';
    }
    try {
      const [listData, rtData] = await Promise.all([
        tauriInvoke('accounts_list'),
        tauriInvoke('accounts_runtime_status').catch(() => null),
      ]);
      state.data = listData;
      if (rtData) state.runtime = rtData;
      if (state.data?.error && !silent) {
        showNotice(state.data.error, 'warn');
      }
      render();
    } catch (err) {
      if (!silent) {
        showNotice(`读取账号列表失败: ${err?.message || err}`, 'error');
      }
    } finally {
      state.loading = false;
      if (!silent && els.refreshBtn) {
        els.refreshBtn.textContent = '刷新';
      }
    }
  }

  function startActiveRefresh() {
    stopActiveRefresh();
    state.activeTimer = setInterval(() => {
      if (
        state.active &&
        document.visibilityState === 'visible' &&
        !state.waiting &&
        !state.busy &&
        !state.renamingId &&
        !state.pendingConfirm
      ) {
        refreshList({ silent: true });
      }
    }, ACTIVE_REFRESH_MS);
    state.countdownTimer = setInterval(() => {
      if (
        state.active &&
        document.visibilityState === 'visible' &&
        !state.renamingId &&
        !state.pendingConfirm &&
        state.data
      ) {
        render();
      }
    }, QUOTA_TICK_MS);
  }

  function stopActiveRefresh() {
    if (state.activeTimer) {
      clearInterval(state.activeTimer);
      state.activeTimer = null;
    }
    if (state.countdownTimer) {
      clearInterval(state.countdownTimer);
      state.countdownTimer = null;
    }
  }

  function clearWaitTimers() {
    if (state.waitTimer) {
      clearInterval(state.waitTimer);
      state.waitTimer = null;
    }
    if (state.pollTimer) {
      clearInterval(state.pollTimer);
      state.pollTimer = null;
    }
    state.pollInFlight = false;
  }

  function startWaiting() {
    state.waiting = true;
    state.pollInFlight = false;
    state.waitDeadline = Date.now() + WAIT_TIMEOUT_MS;
    els.waitBar.classList.remove('hidden');
    els.waitFill.style.width = '0%';
    updateTabBadge();

    if (state.previousEmail) {
      els.waitCancelBtn.textContent = `取消并恢复 ${state.previousEmail}`;
      els.waitStopOnlyBtn.classList.remove('hidden');
    } else {
      els.waitCancelBtn.textContent = '停止等待';
      els.waitStopOnlyBtn.classList.add('hidden');
    }
    setBusy(state.busy);

    const tick = () => {
      const elapsed = Math.min(
        WAIT_TIMEOUT_MS,
        Date.now() - (state.waitDeadline - WAIT_TIMEOUT_MS)
      );
      const total = Math.round(WAIT_TIMEOUT_MS / 1000);
      const secs = Math.min(total, Math.round(elapsed / 1000));
      els.waitLabel.textContent = `等待登录 ${secs}/${total}s`;
      els.waitFill.style.width = `${Math.round((elapsed / WAIT_TIMEOUT_MS) * 100)}%`;
      if (elapsed >= WAIT_TIMEOUT_MS) {
        stopWaiting();
        showNotice('等待登录超时：在 Antigravity 登录成功后点击「保存当前账号」即可归档。', 'warn');
        refreshList();
      }
    };
    state.waitTimer = setInterval(tick, 1000);
    tick();

    state.pollTimer = setInterval(async () => {
      if (!state.waiting || state.pollInFlight) return;
      state.pollInFlight = true;
      try {
        const cur = await tauriInvoke('accounts_current');
        if (!state.waiting) return;
        if (cur?.present && cur.email) {
          const prev = state.previousEmail;
          stopWaiting();
          const sameAsPrev = prev && head(prev) === head(cur.email);
          appendLog(
            sameAsPrev
              ? `[OK] 检测到重新登录原账号 ${cur.email}，正在更新凭据...`
              : `[OK] 检测到新登录账号 ${cur.email}，正在自动归档...`
          );
          try {
            const saved = await tauriInvoke('accounts_save_current');
            appendLog(`[OK] 已保存账号 ${saved.email}`);
            showNotice(
              sameAsPrev
                ? `已更新原账号 ${saved.email} 的登录凭据。`
                : `新账号 ${saved.email} 已自动保存，之后可一键切换。`,
              'ok'
            );
          } catch (err) {
            appendLog(`[!] 自动保存失败: ${err?.message || err}`);
            setLogsExpanded(true);
            showNotice('检测到新账号，但自动保存失败；请点「保存当前账号」重试。', 'warn');
          }
          await refreshList();
          window.dispatchEvent(new CustomEvent('agydock:account-switched'));
          maybeBackgroundFetchCurrentQuota(true);
        }
      } catch (err) {
        appendLog(`[!] 轮询登录状态失败: ${err?.message || err}`);
      } finally {
        state.pollInFlight = false;
      }
    }, LOGIN_POLL_MS);
  }

  function stopWaiting() {
    state.waiting = false;
    clearWaitTimers();
    els.waitBar.classList.add('hidden');
    updateTabBadge();
    setBusy(state.busy);
  }

  async function saveCurrent() {
    setBusy(true);
    appendLog('[..] 正在保存当前登录账号...');
    try {
      const saved = await tauriInvoke('accounts_save_current');
      state.healthMap.delete(saved.id);
      appendLog(`[OK] 已保存账号 ${saved.email}`);
      showNotice(`已保存账号 ${saved.email} 的凭据备份`, 'ok');
      await refreshList();
    } catch (err) {
      appendLog(`[X] 保存失败: ${err?.message || err}`);
      setLogsExpanded(true);
      showNotice(`保存失败: ${err?.message || err}`, 'error');
    } finally {
      setBusy(false);
    }
  }

  async function switchTo(id) {
    const acc = state.data?.accounts?.find((a) => a.id === id);
    if (!acc) return;
    const label = acc.name ? `${acc.name} (${acc.email})` : acc.email;
    const msg = state.restartOnSwitch
      ? `将备份当前账号、关闭 Antigravity，把 ${label} 的登录凭据写回系统并重新启动。确认切换？`
      : `将备份当前账号，并把 ${label} 的登录凭据写入 Windows 凭据管理器（不重启 Antigravity）。确认切换？`;
    showConfirm(msg, async () => {
      hideConfirm();
      await doSwitch(id, state.restartOnSwitch);
    });
  }

  async function doSwitch(id, restart = true) {
    const acc = state.data?.accounts?.find((a) => a.id === id);
    setBusy(true);
    setLogsExpanded(true);
    appendLog(
      `[..] 正在切换到 ${acc?.email || id}${restart ? '（切换后重启）' : '（仅写入凭据）'} ...`
    );
    try {
      const res = await tauriInvoke('accounts_switch', { id, restart });
      for (const line of res.logs || []) appendLog(line);
      if (res.ok) {
        showNotice(
          restart
            ? `已切换到 ${res.email || acc?.email}，Antigravity 正在以该账号启动。`
            : `已将 ${res.email || acc?.email} 的凭据写入系统，手动重启 Antigravity 后生效。`,
          'ok'
        );
      } else {
        showNotice(`切换失败: ${res.error || '未知错误'}`, 'error');
      }
    } catch (err) {
      appendLog(`[X] 切换异常: ${err?.message || err}`);
      showNotice(`切换失败: ${err?.message || err}`, 'error');
    } finally {
      await refreshList();
      setBusy(false);
      window.dispatchEvent(new CustomEvent('agydock:account-switched'));
      maybeBackgroundFetchCurrentQuota(true);
    }
  }

  async function addAccount() {
    const hasCurrent = Boolean(state.data?.current?.present);
    const currentEmail = state.data?.current?.email;
    const backupNote = hasCurrent
      ? `会先把当前账号 ${currentEmail || ''} 备份到本地，`
      : '';
    showConfirm(
      `${backupNote}然后关闭 Antigravity、清除当前登录状态并重新启动。请在重新打开的 Antigravity 中登录新账号；登录成功后会自动保存（以后可直接切换，无需再次登录）。确认继续？`,
      async () => {
        hideConfirm();
        await doBeginLogin();
      }
    );
  }

  async function doBeginLogin() {
    setBusy(true);
    setLogsExpanded(true);
    appendLog('[..] 开始添加账号：备份当前账号 → 停止 Antigravity → 清除登录状态 → 重新启动...');
    try {
      const res = await tauriInvoke('accounts_begin_login');
      for (const line of res.logs || []) appendLog(line);
      if (!res.ok) {
        showNotice(`添加账号失败: ${res.error || '未知错误'}`, 'error');
        return;
      }
      state.previousEmail = res.previousEmail || null;
      showNotice('等待登录中：请在 Antigravity 中登录新账号，登录成功后会自动保存。', 'warn');
      startWaiting();
      await refreshList();
    } catch (err) {
      appendLog(`[X] 添加账号异常: ${err?.message || err}`);
      showNotice(`添加账号失败: ${err?.message || err}`, 'error');
    } finally {
      setBusy(false);
    }
  }

  function logoutCurrent() {
    const email = state.data?.current?.email;
    if (!state.data?.current?.present) return;
    showConfirm(
      `确认退出当前账号 ${email || ''}？退出前会自动备份到本地（不会删除已保存的备份），然后关闭 Antigravity 并清除系统凭据。`,
      async () => {
        hideConfirm();
        setBusy(true);
        setLogsExpanded(true);
        appendLog(`[..] 正在退出当前账号 ${email || ''} ...`);
        try {
          const res = await tauriInvoke('accounts_logout', { stopApp: true });
          for (const line of res.logs || []) appendLog(line);
          if (res.ok) {
            showNotice(`已退出 ${res.email || '当前账号'}，本地备份已保留。`, 'ok');
          } else {
            showNotice(`退出登录失败: ${res.error || '未知错误'}`, 'error');
          }
        } catch (err) {
          appendLog(`[X] 退出异常: ${err?.message || err}`);
          showNotice(`退出登录失败: ${err?.message || err}`, 'error');
        } finally {
          await refreshList();
          setBusy(false);
          window.dispatchEvent(new CustomEvent('agydock:account-switched'));
        }
      }
    );
  }

  async function cancelWaiting({ restore = true } = {}) {
    stopWaiting();
    const previousEmail = state.previousEmail;
    state.previousEmail = null;
    if (!restore || !previousEmail) {
      appendLog('[i] 已停止等待登录。');
      showNotice('已停止等待登录；如需归档新账号，登录完成后点「保存当前账号」即可。', 'warn');
      return;
    }
    const acc = state.data?.accounts?.find((a) => a.email === head(previousEmail));
    if (!acc) {
      appendLog('[i] 已停止等待登录（找不到原账号的备份，未执行恢复）。');
      showNotice('已停止等待登录；如需恢复原账号请稍后手动切换。', 'warn');
      return;
    }
    appendLog(`[..] 取消添加账号，正在恢复原账号 ${acc.email} ...`);
    await doSwitch(acc.id, true);
  }

  async function togglePin(id) {
    const acc = state.data?.accounts?.find((a) => a.id === id);
    if (!acc) return;
    try {
      const updated = await tauriInvoke('accounts_toggle_pin', {
        id,
        pinned: !acc.pinned,
      });
      appendLog(
        updated.pinned
          ? `[OK] 已置顶账号 ${updated.email}`
          : `[i] 已取消置顶 ${updated.email}`
      );
      await refreshList({ silent: true });
    } catch (err) {
      appendLog(`[X] 置顶操作失败: ${err?.message || err}`);
      showNotice(`置顶操作失败: ${err?.message || err}`, 'error');
    }
  }

  async function verifyAllBackups() {
    setBusy(true);
    appendLog('[..] 正在使用 DPAPI 深度校验全部本地账号备份...');
    try {
      const results = await tauriInvoke('accounts_verify');
      state.healthMap.clear();
      let okCount = 0;
      let badCount = 0;
      for (const item of results || []) {
        state.healthMap.set(item.id, item);
        if (item.status === 'ok') {
          okCount += 1;
        } else {
          badCount += 1;
          appendLog(`[!] 备份异常 ${item.email}: ${item.detail || item.status}`);
        }
      }
      if (!results || !results.length) {
        appendLog('[i] 当前没有可校验的账号备份。');
        showNotice('当前没有已保存的账号备份。', 'warn');
      } else if (badCount === 0) {
        appendLog(`[OK] 校验完成：全部 ${okCount} 个账号备份均可正常解密。`);
        showNotice(`校验通过：全部 ${okCount} 个账号备份完好。`, 'ok');
      } else {
        setLogsExpanded(true);
        showNotice(`校验完成：${okCount} 个正常，${badCount} 个备份异常（请查看日志）。`, 'warn');
      }
      render();
    } catch (err) {
      appendLog(`[X] 备份校验失败: ${err?.message || err}`);
      setLogsExpanded(true);
      showNotice(`备份校验失败: ${err?.message || err}`, 'error');
    } finally {
      setBusy(false);
    }
  }

  async function openStoreDir() {
    try {
      const dir = await tauriInvoke('accounts_open_store_dir');
      appendLog(`[i] 已打开账号目录: ${dir}`);
    } catch (err) {
      appendLog(`[X] 无法打开账号目录: ${err?.message || err}`);
      showNotice(`打开目录失败: ${err?.message || err}`, 'error');
    }
  }

  async function renameAccount(id) {
    state.renamingId = id;
    render();
  }

  async function commitRename(id, value) {
    state.renamingId = null;
    try {
      const updated = await tauriInvoke('accounts_rename', { id, name: value });
      if (state.data) {
        const entry = state.data.accounts.find((a) => a.id === id);
        if (entry) entry.name = updated.name;
      }
      render();
    } catch (err) {
      appendLog(`[X] 保存备注失败: ${err?.message || err}`);
      showNotice(`保存备注失败: ${err?.message || err}`, 'error');
      render();
    }
  }

  function removeAccount(id) {
    const acc = state.data?.accounts?.find((a) => a.id === id);
    showConfirm(
      `确认删除账号 ${acc?.email || id} 的本地凭据备份？${
        isCurrent(acc?.email) ? '该账号当前仍处于登录状态，删除备份不会退出登录。' : ''
      }${state.data?.accounts?.length === 1 ? '（这是最后一个备份）' : ''}`,
      async () => {
        hideConfirm();
        setBusy(true);
        try {
          const removed = await tauriInvoke('accounts_remove', { id });
          state.healthMap.delete(id);
          appendLog(removed ? `[OK] 已删除账号备份 ${acc?.email || id}` : '[i] 该账号已不存在。');
          showNotice(removed ? `已删除账号备份 ${acc?.email || id}` : '该账号已不存在', 'ok');
          await refreshList();
          window.dispatchEvent(new CustomEvent('agydock:account-switched'));
          maybeBackgroundFetchCurrentQuota(true);
        } catch (err) {
          appendLog(`[X] 删除失败: ${err?.message || err}`);
          showNotice(`删除失败: ${err?.message || err}`, 'error');
        } finally {
          setBusy(false);
        }
      }
    );
  }

  function emitQuotaUpdated(snapshots) {
    const valid = (snapshots || []).filter((s) => s && s.email && s.email !== 'unknown');
    if (!valid.length) return;
    window.dispatchEvent(
      new CustomEvent('agydock:quota-updated', {
        detail: { source: 'accounts', snapshots: valid },
      })
    );
  }

  async function fetchQuotaForAccount(id, { expand = true, forceRefresh = true } = {}) {
    if (state.quotaLoadingIds.has(id)) return;
    state.quotaLoadingIds.add(id);
    if (expand) {
      state.expandedQuotaId = id;
    }
    render();
    try {
      const snap = await tauriInvoke('quota_fetch_account', { id, forceRefresh });
      const key = normalizeEmailKey(snap?.email);
      if (key && key !== 'unknown') {
        state.quotaMap.set(key, snap);
        emitQuotaUpdated([snap]);
      }
      if (snap?.error) {
        appendLog(`[!] 查询 ${snap.email || id} 额度异常: ${snap.error}`);
      } else {
        const tier = snap?.plan?.tierLabel || '未知';
        const minPct = formatRemainingPct(snap?.minRemainingFraction);
        appendLog(`[OK] ${snap?.email || id}: ${tier} · 最低剩余 ${minPct}`);
      }
    } catch (err) {
      appendLog(`[X] 查询账号额度失败: ${err?.message || err}`);
      showNotice(`查询额度失败: ${err?.message || err}`, 'error');
    } finally {
      state.quotaLoadingIds.delete(id);
      render();
    }
  }

  async function fetchAllQuotas() {
    if (state.quotaFetchingAll) return;
    state.quotaFetchingAll = true;
    if (els.quotaAllBtn) els.quotaAllBtn.textContent = '查询中...';
    render();
    appendLog('[..] 正在批量查询全部已保存账号的套餐等级与剩余额度...');
    try {
      const results = await tauriInvoke('quota_fetch_all');
      let okCount = 0;
      let errCount = 0;
      const updated = [];
      for (const snap of results || []) {
        const key = normalizeEmailKey(snap?.email);
        if (key && key !== 'unknown') {
          state.quotaMap.set(key, snap);
          updated.push(snap);
        }
        if (snap?.error) {
          errCount += 1;
          appendLog(`[!] ${snap.email || '未知账号'} 查询异常: ${snap.error}`);
        } else {
          okCount += 1;
        }
      }
      emitQuotaUpdated(updated);
      if (!results || !results.length) {
        appendLog('[i] 当前没有可查询的已保存账号。');
        showNotice('当前没有可查询的账号。', 'warn');
      } else if (errCount === 0) {
        appendLog(`[OK] 额度刷新完成：共更新 ${okCount} 个账号。`);
        showNotice(`已更新 ${okCount} 个账号的套餐与剩余额度。`, 'ok');
      } else {
        setLogsExpanded(true);
        showNotice(`额度刷新完成：${okCount} 个成功，${errCount} 个异常（详情见日志）。`, 'warn');
      }
    } catch (err) {
      appendLog(`[X] 批量查询额度失败: ${err?.message || err}`);
      setLogsExpanded(true);
      showNotice(`批量查询额度失败: ${err?.message || err}`, 'error');
    } finally {
      state.quotaFetchingAll = false;
      if (els.quotaAllBtn) els.quotaAllBtn.textContent = '刷新全部额度';
      render();
    }
  }

  async function maybeBackgroundFetchCurrentQuota(force = false) {
    const email = state.data?.current?.email;
    if (!state.data?.current?.present || !email) return;
    const key = normalizeEmailKey(email);
    const existing = state.quotaMap.get(key);
    const isFresh =
      existing && !existing.error && existing.fetchedAtMs && Date.now() - existing.fetchedAtMs < QUOTA_STALE_MS;
    if (!force && isFresh) return;
    try {
      const snap = await tauriInvoke('quota_fetch_current', { forceRefresh: false });
      const snapKey = normalizeEmailKey(snap?.email || email);
      if (snapKey && snapKey !== 'unknown') {
        state.quotaMap.set(snapKey, snap);
        emitQuotaUpdated([snap]);
        render();
      }
    } catch {
      // ignore background failure
    }
  }

  function attachRowHandlers() {
    if (!state._rowsWired) {
      state._rowsWired = true;
      els.tableBody.addEventListener('click', async (evt) => {
        const copyEl = evt.target.closest('[data-copy-email]');
        if (copyEl) {
          await copyText(copyEl.dataset.copyEmail, '邮箱');
          return;
        }
        const nameSpan = evt.target.closest('[data-act="rename"]');
        if (nameSpan) {
          await renameAccount(nameSpan.dataset.id);
          return;
        }
        const btn = evt.target.closest('button[data-act]');
        if (btn) {
          if (btn.disabled) return;
          const id = btn.dataset.id;
          if (btn.dataset.act === 'switch') await switchTo(id);
          else if (btn.dataset.act === 'quota' || btn.dataset.act === 'quota-refresh') {
            await fetchQuotaForAccount(id, { expand: true, forceRefresh: true });
          } else if (btn.dataset.act === 'pin') await togglePin(id);
          else if (btn.dataset.act === 'remove') removeAccount(id);
          else if (btn.dataset.act === 'rename-btn') await renameAccount(id);
          return;
        }
        const toggleCell = evt.target.closest('[data-act="toggle-quota"]');
        if (toggleCell) {
          const id = toggleCell.dataset.id;
          if (state.expandedQuotaId === id) {
            state.expandedQuotaId = null;
            render();
            return;
          }
          state.expandedQuotaId = id;
          render();
          const acc = state.data?.accounts?.find((a) => a.id === id);
          if (acc && !state.quotaMap.get(normalizeEmailKey(acc.email))) {
            await fetchQuotaForAccount(id, { expand: true, forceRefresh: false });
          }
        }
      });
      els.tableBody.addEventListener('keydown', (evt) => {
        const input = evt.target.closest('[data-rename-input]');
        if (!input) return;
        if (evt.key === 'Enter') {
          evt.preventDefault();
          commitRename(input.dataset.renameInput, input.value);
        } else if (evt.key === 'Escape') {
          evt.preventDefault();
          state.renamingId = null;
          render();
        }
      });
      els.tableBody.addEventListener(
        'focusout',
        (evt) => {
          const input = evt.target.closest('[data-rename-input]');
          if (!input) return;
          if (state.renamingId === input.dataset.renameInput) {
            commitRename(input.dataset.renameInput, input.value);
          }
        },
        true
      );
    }
  }

  // Toolbar & banner bindings
  els.addBtn.addEventListener('click', addAccount);
  els.saveBtn.addEventListener('click', saveCurrent);
  els.logoutBtn.addEventListener('click', logoutCurrent);
  els.quotaAllBtn?.addEventListener('click', fetchAllQuotas);
  els.refreshBtn.addEventListener('click', () => refreshList());
  els.detectSaveBtn.addEventListener('click', saveCurrent);
  els.noticeCloseBtn.addEventListener('click', () => showNotice(''));

  els.currentEmail.addEventListener('click', () => {
    const email = state.data?.current?.email;
    if (email) copyText(email, '当前账号邮箱');
  });

  els.restartChk.addEventListener('change', () => {
    state.restartOnSwitch = Boolean(els.restartChk.checked);
    saveRestartPref(state.restartOnSwitch);
    render();
  });

  els.searchInput.addEventListener('input', () => {
    state.search = els.searchInput.value;
    if (state.data) {
      renderTable(state.data);
      setBusy(state.busy);
    }
  });

  els.sortSelect.addEventListener('change', () => {
    state.sort = els.sortSelect.value;
    if (state.data) {
      renderTable(state.data);
      setBusy(state.busy);
    }
  });

  els.verifyBtn.addEventListener('click', verifyAllBackups);
  els.openDirBtn.addEventListener('click', openStoreDir);

  function applyOAuthStatus(status) {
    if (!status) return;
    const src = status.source || 'none';
    const configured = Boolean(status.configured);

    if (els.oauthBtn) {
      els.oauthBtn.textContent = configured ? '离线续期 ✓' : '离线续期';
    }
    if (els.oauthStatusTag) {
      let label = '未配置';
      if (src === 'dpapi') label = '已配置 · DPAPI';
      else if (src === 'env') label = '已配置 · 环境变量';
      else if (src === 'file') label = '已配置 · JSON';
      else if (configured) label = '已配置';
      els.oauthStatusTag.textContent = label;
      els.oauthStatusTag.className = configured ? 'tag mono tag-ok' : 'tag mono';
    }
    if (els.oauthClientId) {
      els.oauthClientId.placeholder =
        configured && status.clientIdPreview
          ? `当前已配置 ${status.clientIdPreview}（留空保持不变，重新输入可覆盖）`
          : '粘贴 OAuth Client ID（可选）';
    }
    if (els.oauthClientSecret) {
      els.oauthClientSecret.placeholder = configured
        ? '已保存（不回显；重新输入并保存可覆盖）'
        : '粘贴 OAuth Client Secret（保存时经 DPAPI 加密，不回显原文）';
    }
    if (els.oauthPathNote) {
      els.oauthPathNote.textContent =
        src === 'env'
          ? `优先使用环境变量 AGYDOCK_OAUTH_CLIENT_ID / AGYDOCK_OAUTH_CLIENT_SECRET（本机加密路径: ${
              status.configPath || '—'
            }）`
          : status.configPath || '—';
    }
    if (els.oauthClearBtn) {
      els.oauthClearBtn.classList.toggle('hidden', src !== 'dpapi' && src !== 'file');
    }
  }

  async function loadOAuthStatus() {
    if (!els.oauthPanel) return;
    try {
      const status = await tauriInvoke('quota_oauth_config_status');
      applyOAuthStatus(status);
    } catch {
      // ignore initial status read failures
    }
  }

  async function saveOAuthConfig() {
    if (!els.oauthSaveBtn) return;
    const clientId = String(els.oauthClientId?.value ?? '').trim();
    const clientSecret = String(els.oauthClientSecret?.value ?? '').trim();
    if (!clientId || !clientSecret) {
      showNotice('请同时填写 Client ID 和 Client Secret。', 'warn');
      if (!clientId) els.oauthClientId?.focus();
      else els.oauthClientSecret?.focus();
      return;
    }

    els.oauthSaveBtn.disabled = true;
    if (els.oauthClearBtn) els.oauthClearBtn.disabled = true;
    const prevText = els.oauthSaveBtn.textContent;
    els.oauthSaveBtn.textContent = '保存中...';
    try {
      const status = await tauriInvoke('quota_oauth_config_save', { clientId, clientSecret });
      if (els.oauthClientId) els.oauthClientId.value = '';
      if (els.oauthClientSecret) els.oauthClientSecret.value = '';
      applyOAuthStatus(status);
      appendLog(
        `[OK] 已使用 Windows DPAPI 加密保存本地 OAuth 刷新凭据 (${status?.clientIdPreview || '已配置'})。`
      );
      showNotice('已加密保存离线续期配置；现在可随时查询离线账号额度。', 'ok');
    } catch (err) {
      appendLog(`[X] 保存离线续期配置失败: ${err?.message || err}`);
      showNotice(`保存配置失败: ${err?.message || err}`, 'error');
    } finally {
      els.oauthSaveBtn.disabled = false;
      els.oauthSaveBtn.textContent = prevText;
      if (els.oauthClearBtn) els.oauthClearBtn.disabled = false;
    }
  }

  async function clearOAuthConfig() {
    if (!els.oauthClearBtn) return;
    els.oauthClearBtn.disabled = true;
    if (els.oauthSaveBtn) els.oauthSaveBtn.disabled = true;
    try {
      const status = await tauriInvoke('quota_oauth_config_clear');
      if (els.oauthClientId) els.oauthClientId.value = '';
      if (els.oauthClientSecret) els.oauthClientSecret.value = '';
      applyOAuthStatus(status);
      appendLog('[OK] 已清除本机 OAuth 刷新凭据文件。');
      showNotice(
        status?.source === 'env'
          ? '已清除本地凭据文件（环境变量配置仍生效）。'
          : '已清除离线续期配置。',
        'ok'
      );
    } catch (err) {
      appendLog(`[X] 清除离线续期配置失败: ${err?.message || err}`);
      showNotice(`清除配置失败: ${err?.message || err}`, 'error');
    } finally {
      els.oauthClearBtn.disabled = false;
      if (els.oauthSaveBtn) els.oauthSaveBtn.disabled = false;
    }
  }

  els.oauthBtn?.addEventListener('click', () => {
    if (!els.oauthPanel) return;
    const willShow = els.oauthPanel.classList.contains('hidden');
    els.oauthPanel.classList.toggle('hidden', !willShow);
    if (willShow) {
      loadOAuthStatus();
    }
  });
  els.oauthCloseBtn?.addEventListener('click', () => {
    els.oauthPanel?.classList.add('hidden');
  });
  els.oauthSaveBtn?.addEventListener('click', saveOAuthConfig);
  els.oauthClearBtn?.addEventListener('click', clearOAuthConfig);

  els.confirmOkBtn.addEventListener('click', () => {
    const fn = state.pendingConfirm;
    hideConfirm();
    if (fn) fn();
  });
  els.confirmCancelBtn.addEventListener('click', () => {
    hideConfirm();
    appendLog('[i] 已取消操作。');
  });
  els.waitCancelBtn.addEventListener('click', () => cancelWaiting({ restore: true }));
  els.waitStopOnlyBtn.addEventListener('click', () => cancelWaiting({ restore: false }));

  els.logToggleBtn.addEventListener('click', () => setLogsExpanded(!state.logsExpanded));

  els.logCopyBtn.addEventListener('click', async () => {
    if (!state.logs.length) return;
    try {
      await navigator.clipboard.writeText(state.logs.join('\n'));
      const original = els.logCopyBtn.textContent;
      els.logCopyBtn.textContent = '已复制';
      setTimeout(() => {
        els.logCopyBtn.textContent = original;
      }, 1200);
    } catch {
      appendLog('[!] 无法写入剪贴板。');
    }
  });
  els.logClearBtn.addEventListener('click', () => {
    state.logs = [];
    els.logPre.textContent = '';
    els.logCount.textContent = '0';
    els.logPreview.textContent = '日志已清空';
  });

  async function ensureLoaded() {
    if (state.initialized) return;
    state.initialized = true;
    appendLog('[i] 就绪：凭据存于 Windows 凭据管理器，本地备份使用 DPAPI 加密，托盘右键可快速切换。');
    window.addEventListener('agydock:quota-updated', (evt) => {
      const detail = evt.detail || {};
      if (detail.source === 'accounts') return;
      let changed = false;
      for (const snap of detail.snapshots || []) {
        const key = normalizeEmailKey(snap?.email);
        if (key && key !== 'unknown') {
          state.quotaMap.set(key, snap);
          changed = true;
        }
      }
      if (changed && state.data) {
        render();
      }
    });
    if (tauriListen && !state.unlistenTray) {
      try {
        state.unlistenTray = await tauriListen('accounts-changed', (event) => {
          const payload = event.payload || {};
          for (const line of payload.logs || []) appendLog(line);
          if (payload.ok) {
            showNotice(`已通过托盘切换到 ${payload.email || '目标账号'}。`, 'ok');
          } else if (payload.error) {
            setLogsExpanded(true);
            showNotice(`托盘切换失败: ${payload.error}`, 'error');
          }
          refreshList({ silent: true }).then(() => maybeBackgroundFetchCurrentQuota(true));
        });
      } catch {
        // ignore event registration failures
      }
    }
    loadOAuthStatus();
    try {
      const cachedMap = await tauriInvoke('quota_get_cached').catch(() => ({}));
      for (const [k, v] of Object.entries(cachedMap || {})) {
        state.quotaMap.set(normalizeEmailKey(k), v);
      }
    } catch {
      // ignore cache load errors
    }
    await refreshList();
    maybeBackgroundFetchCurrentQuota(false);
  }

  return {
    setActive(active) {
      state.active = active;
      if (active) {
        ensureLoaded().then(() => {
          refreshList({ silent: true });
          startActiveRefresh();
        });
      } else {
        stopActiveRefresh();
      }
    },
  };
}
