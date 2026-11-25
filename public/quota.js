const tauriInvoke =
  typeof window !== 'undefined' && typeof window.__TAURI__?.core?.invoke === 'function'
    ? window.__TAURI__.core.invoke
    : null;

const tauriListen =
  typeof window !== 'undefined' && typeof window.__TAURI__?.event?.listen === 'function'
    ? window.__TAURI__.event.listen
    : null;

const COUNTDOWN_TICK_MS = 30_000;
const AUTO_REFRESH_MS = 180_000;

export function escapeHtml(str) {
  return String(str ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function pad2(n) {
  return String(n).padStart(2, '0');
}

export function normalizeEmailKey(email) {
  return String(email ?? '').trim().toLowerCase();
}

export function formatRemainingPct(fraction) {
  if (fraction === null || fraction === undefined || Number.isNaN(Number(fraction))) {
    return '—';
  }
  const f = Math.max(0, Math.min(1, Number(fraction)));
  if (f === 0) return '0%';
  if (f < 0.005) return '<1%';
  return `${Math.round(f * 100)}%`;
}

export function quotaToneClass(fraction) {
  if (fraction === null || fraction === undefined || Number.isNaN(Number(fraction))) {
    return 'muted';
  }
  const f = Number(fraction);
  if (f >= 0.5) return 'ok';
  if (f >= 0.2) return 'warn';
  return 'err';
}

export function formatResetCountdown(resetAtMs, nowMs = Date.now()) {
  if (!resetAtMs) return '—';
  const diffMs = Number(resetAtMs) - nowMs;
  if (!Number.isFinite(diffMs)) return '—';
  if (diffMs <= 0) return '已刷新/待同步';

  const totalMin = Math.floor(diffMs / 60_000);
  if (totalMin < 1) return '<1m 后刷新';
  if (totalMin < 60) return `${totalMin}m 后刷新`;

  const totalHours = Math.floor(totalMin / 60);
  const remMin = totalMin % 60;
  if (totalHours < 24) {
    return remMin > 0 ? `${totalHours}h ${remMin}m 后刷新` : `${totalHours}h 后刷新`;
  }

  const days = Math.floor(totalHours / 24);
  const remHours = totalHours % 24;
  return remHours > 0 ? `${days}d ${remHours}h 后刷新` : `${days}d 后刷新`;
}

export function formatResetExact(resetAtMs) {
  if (!resetAtMs) return '—';
  const d = new Date(Number(resetAtMs));
  if (Number.isNaN(d.getTime())) return '—';
  return `${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

export function formatTimeClock(ms) {
  if (!ms) return '—';
  const d = new Date(Number(ms));
  if (Number.isNaN(d.getTime())) return '—';
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

export function formatCreditsSummary(plan) {
  const credits = Array.isArray(plan?.credits) ? plan.credits : [];
  if (!credits.length) return '';
  const first = credits[0];
  if (first.amount) {
    return `AI Credits: ${first.amount}`;
  }
  if (first.minForUsage) {
    return `支持 AI Credits (≥${first.minForUsage})`;
  }
  return '支持 AI Credits';
}

export function tierBadgeHtml(plan) {
  if (!plan) return '<span class="tag tag-tier tag-tier-unknown">—</span>';
  const code = plan.tierCode || 'unknown';
  const shortLabel =
    code === 'ultra'
      ? 'ULTRA'
      : code === 'pro'
        ? 'PRO'
        : code === 'standard'
          ? 'STANDARD'
          : code === 'free'
            ? 'FREE'
            : plan.tierLabel || '未知';
  const titleParts = [plan.tierLabel || shortLabel];
  if (plan.tierId) titleParts.push(`ID: ${plan.tierId}`);
  if (plan.refreshCadenceHint) titleParts.push(plan.refreshCadenceHint);
  return `<span class="tag tag-tier tag-tier-${escapeHtml(code)}" title="${escapeHtml(
    titleParts.join(' · ')
  )}">${escapeHtml(shortLabel)}</span>`;
}

function formatWindowPill(win) {
  if (!win) return '';
  const lower = String(win).toLowerCase();
  const text = lower === 'weekly' || lower === '7d' ? '周额度' : win;
  return `<span class="tag">${escapeHtml(text)}</span>`;
}

function renderBucketsSectionHtml(buckets) {
  if (!Array.isArray(buckets) || !buckets.length) return '';
  const cards = buckets
    .map((b) => {
      const pctText = formatRemainingPct(b.remainingFraction);
      const tone = quotaToneClass(b.remainingFraction);
      const width =
        b.remainingFraction === null || b.remainingFraction === undefined
          ? 0
          : Math.round(Math.max(0, Math.min(1, Number(b.remainingFraction))) * 100);
      const resetText = formatResetCountdown(b.resetAtMs);
      const exactText = formatResetExact(b.resetAtMs);
      return `
        <div class="quota-bucket-card">
          <div class="quota-bucket-top">
            <span class="quota-bucket-name">${escapeHtml(b.groupLabel)} · ${escapeHtml(b.label)}</span>
            ${formatWindowPill(b.window)}
            <span class="quota-bucket-pct mono tone-${tone}">${pctText}</span>
          </div>
          <div class="quota-bar-track"><div class="quota-bar-fill tone-${tone}" style="width:${width}%"></div></div>
          <div class="quota-pool-sub mono">
            <span>${escapeHtml(resetText)}</span>
            <span>${escapeHtml(exactText)}</span>
          </div>
        </div>
      `;
    })
    .join('');

  return `
    <div class="detail-section-title">额度周期窗口 (${buckets.length})</div>
    <div class="quota-bucket-grid">${cards}</div>
  `;
}

export function renderModelsTableHtml(models) {
  if (!Array.isArray(models) || !models.length) {
    return '<div class="panel-note" style="padding: 8px 0;">暂无模型额度明细</div>';
  }
  const rows = models
    .map((m) => {
      const pctText = formatRemainingPct(m.remainingFraction);
      const tone = quotaToneClass(m.remainingFraction);
      const width =
        m.remainingFraction === null || m.remainingFraction === undefined
          ? 0
          : Math.round(Math.max(0, Math.min(1, Number(m.remainingFraction))) * 100);
      const recTag = m.recommended ? '<span class="tag">推荐</span>' : '';
      return `
        <tr>
          <td>
            <div class="session-title-row">
              <span class="mono">${escapeHtml(m.displayName || m.modelId)}</span>
              ${recTag}
            </div>
            <div class="session-id">${escapeHtml(m.modelId)}</div>
          </td>
          <td><span class="tag">${escapeHtml(m.poolLabel || m.poolKey)}</span></td>
          <td>
            <div class="quota-table-bar">
              <div class="quota-bar-track"><div class="quota-bar-fill tone-${tone}" style="width:${width}%"></div></div>
              <span class="mono tone-${tone}">${pctText}</span>
            </div>
          </td>
          <td class="mono">${escapeHtml(formatResetCountdown(m.resetAtMs))}</td>
          <td class="mono">${escapeHtml(formatResetExact(m.resetAtMs))}</td>
        </tr>
      `;
    })
    .join('');

  return `
    <div class="table-wrap">
      <table class="sub-table">
        <thead>
          <tr>
            <th>模型</th>
            <th style="width: 112px;">额度分组</th>
            <th style="width: 170px;">剩余额度</th>
            <th style="width: 126px;">刷新倒计时</th>
            <th style="width: 110px;">刷新时间</th>
          </tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
  `;
}

export function renderQuotaDetailHtml(snapshot, { showRefreshBtn = false, accountId = '' } = {}) {
  if (!snapshot) {
    return '<div class="detail-box"><span class="panel-note">暂无额度信息，点击「刷新」获取</span></div>';
  }

  const plan = snapshot.plan || {};
  const creditsText = formatCreditsSummary(plan);
  const metaItems = [
    `<span>套餐: <strong>${escapeHtml(plan.tierLabel || '未知')}</strong>${
      plan.tierId ? ` (${escapeHtml(plan.tierId)})` : ''
    }</span>`,
  ];
  if (plan.refreshCadenceHint) {
    metaItems.push(`<span>周期: ${escapeHtml(plan.refreshCadenceHint)}</span>`);
  }
  if (creditsText) {
    metaItems.push(`<span>${escapeHtml(creditsText)}</span>`);
  }
  if (snapshot.projectId) {
    metaItems.push(`<span>Project: ${escapeHtml(snapshot.projectId)}</span>`);
  }
  metaItems.push(
    `<span>通道: ${escapeHtml(snapshot.viaProxy ? `隔离代理 ${snapshot.viaProxy}` : '系统网络/直连')}</span>`
  );
  metaItems.push(`<span>查询于 ${escapeHtml(formatTimeClock(snapshot.fetchedAtMs))}</span>`);

  const refreshBtnHtml = showRefreshBtn
    ? `<button type="button" class="btn" data-act="quota-refresh" data-id="${escapeHtml(
        accountId
      )}">重新查询</button>`
    : '';

  const notices = [];
  if (snapshot.error) {
    notices.push(`<div class="notice notice-error">${escapeHtml(snapshot.error)}</div>`);
  }
  if (plan.ineligibleReason) {
    notices.push(`<div class="notice">账号受限提示: ${escapeHtml(plan.ineligibleReason)}</div>`);
  }

  const bucketsHtml = renderBucketsSectionHtml(snapshot.summaryBuckets);
  const modelsCount = Array.isArray(snapshot.models) ? snapshot.models.length : 0;

  return `
    <div class="detail-box">
      <div class="detail-meta mono quota-detail-meta">
        <div class="quota-detail-meta-items">${metaItems.join(' · ')}</div>
        ${refreshBtnHtml}
      </div>
      ${notices.join('')}
      ${bucketsHtml}
      <div class="detail-section-title">模型额度明细 (${modelsCount})</div>
      ${renderModelsTableHtml(snapshot.models)}
    </div>
  `;
}

export function initUsageQuotaPanel() {
  const panel = document.getElementById('quotaPanel');
  if (!tauriInvoke || !panel) {
    return {
      refresh() {},
      setActive() {},
      onAccountChanged() {},
    };
  }

  const els = {
    panel,
    tierBadge: document.getElementById('quotaTierBadge'),
    accountEmail: document.getElementById('quotaAccountEmail'),
    creditsTag: document.getElementById('quotaCreditsTag'),
    toggleModelsBtn: document.getElementById('quotaToggleModelsBtn'),
    refreshBtn: document.getElementById('quotaRefreshBtn'),
    poolsStage: document.getElementById('quotaPoolsStage'),
    modelsWrap: document.getElementById('quotaModelsWrap'),
    metaLeft: document.getElementById('quotaMetaLeft'),
    metaRight: document.getElementById('quotaMetaRight'),
  };

  panel.classList.remove('hidden');

  const state = {
    active: true,
    loading: false,
    modelsExpanded: false,
    currentEmail: null,
    present: false,
    snapshot: null,
    countdownTimer: null,
    autoTimer: null,
    initialized: false,
  };

  function emitQuotaUpdated(snapshot) {
    if (!snapshot || !snapshot.email || snapshot.email === 'unknown') return;
    window.dispatchEvent(
      new CustomEvent('agydock:quota-updated', {
        detail: { source: 'usage', snapshots: [snapshot] },
      })
    );
  }

  function render() {
    const snap = state.snapshot;
    const email = snap?.email && snap.email !== 'unknown' ? snap.email : state.currentEmail;

    if (!state.present && !snap) {
      els.tierBadge.innerHTML = '<span class="tag tag-tier tag-tier-unknown">未登录</span>';
      els.accountEmail.textContent = '未检测到登录凭据';
      els.creditsTag.classList.add('hidden');
      els.toggleModelsBtn.classList.add('hidden');
      els.poolsStage.innerHTML =
        '<div class="quota-empty">当前未登录 Antigravity 账号，请在客户端登录或在「账号管理」页切换账号。</div>';
      els.modelsWrap.classList.add('hidden');
      els.metaLeft.textContent = '等待登录账号';
      els.metaRight.textContent = '';
      return;
    }

    if (!snap) {
      els.tierBadge.innerHTML = '<span class="tag tag-tier tag-tier-unknown">查询中</span>';
      els.accountEmail.textContent = email || '读取中...';
      els.creditsTag.classList.add('hidden');
      els.toggleModelsBtn.classList.add('hidden');
      els.poolsStage.innerHTML = '<div class="quota-empty">正在读取套餐与剩余额度...</div>';
      els.modelsWrap.classList.add('hidden');
      els.metaLeft.textContent = '正在连接 Cloud Code Assist...';
      els.metaRight.textContent = '';
      return;
    }

    const plan = snap.plan || {};
    els.tierBadge.innerHTML = `${tierBadgeHtml(plan)}${
      plan.tierLabel && plan.tierCode !== 'unknown'
        ? `<span class="panel-note">${escapeHtml(plan.tierLabel)}</span>`
        : ''
    }`;
    els.accountEmail.textContent = email || '未知账号';

    const creditsSummary = formatCreditsSummary(plan);
    if (creditsSummary) {
      els.creditsTag.textContent = creditsSummary;
      els.creditsTag.classList.remove('hidden');
    } else {
      els.creditsTag.classList.add('hidden');
    }

    const models = Array.isArray(snap.models) ? snap.models : [];
    const buckets = Array.isArray(snap.summaryBuckets) ? snap.summaryBuckets : [];
    if (models.length > 0 || buckets.length > 0) {
      els.toggleModelsBtn.classList.remove('hidden');
      els.toggleModelsBtn.textContent = state.modelsExpanded
        ? `收起明细 (${models.length})`
        : `模型明细 (${models.length})`;
    } else {
      els.toggleModelsBtn.classList.add('hidden');
    }

    const pools = Array.isArray(snap.pools) ? snap.pools : [];
    if (pools.length > 0) {
      els.poolsStage.innerHTML = `
        <div class="quota-pool-grid">
          ${pools
            .map((p) => {
              const pct = formatRemainingPct(p.remainingFraction);
              const tone = quotaToneClass(p.remainingFraction);
              const width =
                p.remainingFraction === null || p.remainingFraction === undefined
                  ? 0
                  : Math.round(Math.max(0, Math.min(1, Number(p.remainingFraction))) * 100);
              const countdown = formatResetCountdown(p.resetAtMs);
              const exact = formatResetExact(p.resetAtMs);
              const subBadge = p.window
                ? formatWindowPill(p.window)
                : p.modelCount > 0
                  ? `<span class="tag">${p.modelCount} 模型</span>`
                  : '';
              return `
                <div class="quota-pool-card">
                  <div class="quota-pool-head">
                    <span class="quota-pool-title">${escapeHtml(p.label)}</span>
                    ${subBadge}
                  </div>
                  <div class="quota-pool-pct mono tone-${tone}">${pct}</div>
                  <div class="quota-bar-track"><div class="quota-bar-fill tone-${tone}" style="width:${width}%"></div></div>
                  <div class="quota-pool-sub mono">
                    <span>${escapeHtml(countdown)}</span>
                    <span>${escapeHtml(exact)}</span>
                  </div>
                </div>
              `;
            })
            .join('')}
        </div>
      `;
    } else if (snap.error) {
      els.poolsStage.innerHTML = `<div class="notice notice-error">${escapeHtml(snap.error)}</div>`;
    } else {
      els.poolsStage.innerHTML = '<div class="quota-empty">暂无可用的额度池数据</div>';
    }

    if (state.modelsExpanded && (models.length > 0 || buckets.length > 0)) {
      els.modelsWrap.classList.remove('hidden');
      els.modelsWrap.innerHTML = `
        <div class="quota-models-inner">
          ${renderBucketsSectionHtml(buckets)}
          ${renderModelsTableHtml(models)}
        </div>
      `;
    } else {
      els.modelsWrap.classList.add('hidden');
      els.modelsWrap.innerHTML = '';
    }

    const leftParts = [];
    leftParts.push(snap.viaProxy ? `隔离代理 ${snap.viaProxy}` : '系统网络/直连');
    if (plan.refreshCadenceHint) leftParts.push(plan.refreshCadenceHint);
    if (snap.projectId) leftParts.push(`Project: ${snap.projectId}`);
    if (snap.error && pools.length > 0) leftParts.push(`刷新异常(显示缓存): ${snap.error}`);
    els.metaLeft.textContent = leftParts.join(' · ');
    els.metaRight.textContent = `更新于 ${formatTimeClock(snap.fetchedAtMs)}`;
  }

  async function refresh(forceRefresh = true) {
    if (state.loading) return;
    state.loading = true;
    els.refreshBtn.disabled = true;
    els.refreshBtn.textContent = '查询中...';
    try {
      const cur = await tauriInvoke('accounts_current').catch(() => null);
      state.present = Boolean(cur?.present);
      state.currentEmail = cur?.email || null;
      const snap = await tauriInvoke('quota_fetch_current', { forceRefresh });
      const snapEmail = snap?.email && snap.email !== 'unknown' ? snap.email : null;
      if (!cur?.present && !snapEmail) {
        state.present = false;
        state.currentEmail = null;
        state.snapshot = null;
        render();
        return;
      }
      if (!cur?.present && snapEmail) {
        state.present = true;
        state.currentEmail = snapEmail;
      }
      state.snapshot = snap;
      render();
      emitQuotaUpdated(snap);
    } catch (err) {
      if (state.snapshot) {
        state.snapshot = {
          ...state.snapshot,
          error: `额度查询失败: ${err?.message || err}`,
        };
      } else {
        state.snapshot = {
          email: state.currentEmail || 'unknown',
          fetchedAtMs: Date.now(),
          plan: { tierCode: 'unknown', tierLabel: '未知套餐' },
          pools: [],
          models: [],
          summaryBuckets: [],
          error: `额度查询失败: ${err?.message || err}`,
        };
      }
      render();
    } finally {
      state.loading = false;
      els.refreshBtn.disabled = false;
      els.refreshBtn.textContent = '刷新额度';
    }
  }

  async function bootstrap() {
    if (state.initialized) return;
    state.initialized = true;
    try {
      const [cur, cachedMap] = await Promise.all([
        tauriInvoke('accounts_current').catch(() => null),
        tauriInvoke('quota_get_cached').catch(() => ({})),
      ]);
      state.present = Boolean(cur?.present);
      state.currentEmail = cur?.email || null;
      if (state.currentEmail && cachedMap) {
        const key = normalizeEmailKey(state.currentEmail);
        if (cachedMap[key]) {
          state.snapshot = cachedMap[key];
        }
      }
      render();
    } catch {
      // ignore initial cache read error
    }
    await refresh(false);
  }

  els.refreshBtn.addEventListener('click', () => refresh(true));
  els.toggleModelsBtn.addEventListener('click', () => {
    state.modelsExpanded = !state.modelsExpanded;
    render();
  });

  window.addEventListener('agydock:quota-updated', (evt) => {
    const detail = evt.detail || {};
    if (detail.source === 'usage') return;
    const list = Array.isArray(detail.snapshots) ? detail.snapshots : [];
    const curKey = normalizeEmailKey(state.currentEmail);
    if (!curKey) return;
    const matched = list.find((s) => normalizeEmailKey(s?.email) === curKey);
    if (matched) {
      state.snapshot = matched;
      render();
    }
  });

  window.addEventListener('agydock:account-switched', () => {
    refresh(false);
  });

  if (tauriListen) {
    tauriListen('accounts-changed', () => {
      refresh(false);
    }).catch(() => {});
  }

  function startTimers() {
    stopTimers();
    state.countdownTimer = setInterval(() => {
      if (state.active && document.visibilityState === 'visible' && state.snapshot) {
        render();
      }
    }, COUNTDOWN_TICK_MS);
    state.autoTimer = setInterval(() => {
      if (state.active && document.visibilityState === 'visible') {
        refresh(false);
      }
    }, AUTO_REFRESH_MS);
  }

  function stopTimers() {
    if (state.countdownTimer) {
      clearInterval(state.countdownTimer);
      state.countdownTimer = null;
    }
    if (state.autoTimer) {
      clearInterval(state.autoTimer);
      state.autoTimer = null;
    }
  }

  bootstrap();
  startTimers();

  return {
    refresh(force = false) {
      return refresh(force);
    },
    setActive(active) {
      state.active = active;
      if (active) {
        render();
        startTimers();
      } else {
        stopTimers();
      }
    },
    onAccountChanged() {
      return refresh(false);
    },
  };
}
