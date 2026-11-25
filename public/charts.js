const SVG_NS = 'http://www.w3.org/2000/svg';

function el(tag, attrs = {}, textContent = null) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v !== undefined && v !== null) {
      node.setAttribute(k, String(v));
    }
  }
  if (textContent !== null) {
    node.textContent = textContent;
  }
  return node;
}

export function formatTokensM(n) {
  if (!Number.isFinite(n) || n === 0) return '0M';
  const m = n / 1_000_000;
  const abs = Math.abs(m);
  if (abs >= 100) return `${m.toFixed(1)}M`;
  if (abs >= 0.01) return `${m.toFixed(2)}M`;
  if (abs >= 0.001) return `${m.toFixed(3)}M`;
  return '<0.001M';
}

export function formatAxisTokensM(n) {
  if (!Number.isFinite(n) || n === 0) return '0';
  const m = n / 1_000_000;
  const abs = Math.abs(m);
  const stripZero = (s) => s.replace(/\.?0+$/, '');
  if (abs >= 10) return `${stripZero(m.toFixed(1))}M`;
  if (abs >= 0.1) return `${stripZero(m.toFixed(2))}M`;
  if (abs >= 0.001) return `${stripZero(m.toFixed(3))}M`;
  return `${stripZero(m.toFixed(4))}M`;
}

export function formatExact(n) {
  return Math.round(n || 0).toLocaleString('en-US');
}

export function formatPct(rate) {
  if (!Number.isFinite(rate)) return '0.0%';
  return `${(rate * 100).toFixed(1)}%`;
}

export function formatSpeed(tps) {
  if (!Number.isFinite(tps) || tps <= 0) return '—';
  return tps >= 100 ? `${tps.toFixed(0)} tok/s` : `${tps.toFixed(1)} tok/s`;
}

function niceMax(rawMax) {
  if (rawMax <= 0) return 100;
  const exp = Math.floor(Math.log10(rawMax));
  const base = Math.pow(10, exp);
  const frac = rawMax / base;
  let niceFrac = 1;
  if (frac <= 1) niceFrac = 1;
  else if (frac <= 2) niceFrac = 2;
  else if (frac <= 2.5) niceFrac = 2.5;
  else if (frac <= 5) niceFrac = 5;
  else niceFrac = 10;
  return niceFrac * base;
}

function positionTooltip(tooltipEl, clientX, clientY) {
  const pad = 14;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const rect = tooltipEl.getBoundingClientRect();

  let left = clientX + pad;
  let top = clientY - rect.height / 2;

  if (left + rect.width + 12 > vw) {
    left = clientX - rect.width - pad;
  }
  if (top < 10) top = 10;
  if (top + rect.height + 10 > vh) {
    top = vh - rect.height - 10;
  }

  tooltipEl.style.left = `${Math.round(left)}px`;
  tooltipEl.style.top = `${Math.round(top)}px`;
}

function pickTickStep(count, plotWidth, minLabelPx) {
  if (count <= 1) return 1;
  const maxLabels = Math.max(2, Math.floor(plotWidth / minLabelPx));
  return Math.max(1, Math.ceil(count / maxLabels));
}

export function renderTokenTrendChart({
  container,
  emptyEl,
  tooltipEl,
  series,
  visibleSeries,
  onSelectDay = null,
  onSelectRange = null,
}) {
  if (typeof container._cleanupBrush === 'function') {
    container._cleanupBrush();
    container._cleanupBrush = null;
  }
  const existingSvg = container.querySelector('svg');
  if (existingSvg) existingSvg.remove();

  const hasAnyData = series.some((d) => d.totalTokens > 0);
  if (!series.length || !hasAnyData) {
    emptyEl.classList.remove('hidden');
    return;
  }
  emptyEl.classList.add('hidden');

  const rect = container.getBoundingClientRect();
  const width = Math.max(320, Math.floor(rect.width - 32));
  const height = Math.max(180, Math.floor(rect.height - 20));

  const margin = { top: 10, right: 8, bottom: 26, left: 52 };
  const plotW = Math.max(100, width - margin.left - margin.right);
  const plotH = Math.max(80, height - margin.top - margin.bottom);

  const layerOrder = [
    { key: 'outputTokens', color: '#18181b', label: 'Output' },
    { key: 'inputTokens', color: '#3b6ea5', label: 'Input' },
    { key: 'cacheReadTokens', color: '#cbd5e1', label: 'Cache Read' },
  ].filter((layer) => visibleSeries[layer.key]);

  let rawMax = 0;
  for (const item of series) {
    let sum = 0;
    for (const layer of layerOrder) {
      sum += item[layer.key] || 0;
    }
    if (sum > rawMax) rawMax = sum;
  }
  const yMax = niceMax(rawMax);

  const svg = el('svg', {
    class: 'chart-svg',
    viewBox: `0 0 ${width} ${height}`,
    width,
    height,
  });

  // Gridlines & Y-axis ticks
  const tickCount = 4;
  for (let i = 0; i <= tickCount; i += 1) {
    const ratio = i / tickCount;
    const val = yMax * (1 - ratio);
    const y = Math.round(margin.top + plotH * ratio) + 0.5;

    svg.appendChild(
      el('line', {
        x1: margin.left,
        x2: margin.left + plotW,
        y1: y,
        y2: y,
        stroke: i === tickCount ? '#d4d4d8' : '#ececef',
        'stroke-width': 1,
        'stroke-dasharray': i === tickCount ? null : '2,2',
      })
    );

    svg.appendChild(
      el(
        'text',
        {
          x: margin.left - 8,
          y: y + 3.5,
          'text-anchor': 'end',
          fill: '#71717a',
          'font-size': 10.5,
          'font-family': 'var(--font-mono)',
        },
        formatAxisTokensM(val)
      )
    );
  }

  const slotW = plotW / series.length;
  const barW = Math.max(2, Math.min(34, slotW * 0.68));
  const xStep = pickTickStep(series.length, plotW, 56);

  const barsGroup = el('g');
  const brushGroup = el('g');
  const overlayGroup = el('g');
  const xAxisGroup = el('g');
  const hoverBgs = [];

  series.forEach((item, idx) => {
    const slotX = margin.left + idx * slotW;
    const barX = slotX + (slotW - barW) / 2;

    // Hover background column
    const hoverBg = el('rect', {
      x: slotX,
      y: margin.top,
      width: slotW,
      height: plotH,
      rx: 4,
      ry: 4,
      fill: 'rgba(24, 24, 27, 0.045)',
      opacity: 0,
    });
    barsGroup.appendChild(hoverBg);
    hoverBgs.push(hoverBg);

    let yCursor = margin.top + plotH;
    const activeLayers = layerOrder.filter((layer) => (item[layer.key] || 0) > 0);
    const barRadius = Math.min(3, Math.max(0, barW * 0.25));
    activeLayers.forEach((layer, layerIdx) => {
      const value = item[layer.key] || 0;
      const segH = Math.max(1, (value / yMax) * plotH);
      yCursor -= segH;
      const isTop = layerIdx === activeLayers.length - 1;
      const r = isTop && segH >= 4 ? barRadius : 0;

      barsGroup.appendChild(
        el('rect', {
          x: barX,
          y: yCursor,
          width: barW,
          height: segH,
          rx: r || null,
          ry: r || null,
          fill: layer.color,
        })
      );
    });

    // X-axis labels
    if (idx % xStep === 0 || idx === series.length - 1) {
      const isTooCloseToLast =
        idx !== series.length - 1 && (series.length - 1 - idx) * slotW < 40;
      if (!isTooCloseToLast) {
        xAxisGroup.appendChild(
          el(
            'text',
            {
              x: slotX + slotW / 2,
              y: margin.top + plotH + 17,
              'text-anchor': 'middle',
              fill: '#71717a',
              'font-size': 10.5,
              'font-family': 'var(--font-mono)',
            },
            item.shortLabel
          )
        );
      }
    }
  });

  const brushRect = el('rect', {
    x: margin.left,
    y: margin.top,
    width: 0,
    height: plotH,
    rx: 4,
    ry: 4,
    fill: 'rgba(59, 110, 165, 0.14)',
    stroke: 'rgba(59, 110, 165, 0.65)',
    'stroke-width': 1,
    opacity: 0,
    'pointer-events': 'none',
  });
  brushGroup.appendChild(brushRect);

  const plotHit = el('rect', {
    x: margin.left,
    y: margin.top,
    width: plotW,
    height: plotH,
    fill: 'transparent',
    style: onSelectRange ? 'cursor: crosshair;' : onSelectDay ? 'cursor: pointer;' : 'cursor: crosshair;',
  });

  let hoveredIdx = -1;
  let dragging = false;
  let isBrushing = false;
  let dragStartIdx = -1;
  let dragCurIdx = -1;
  let dragStartX = 0;

  const idxFromClientX = (clientX) => {
    const svgRect = svg.getBoundingClientRect();
    const scaleX = width / Math.max(1, svgRect.width);
    const svgX = (clientX - svgRect.left) * scaleX;
    return Math.max(0, Math.min(series.length - 1, Math.floor((svgX - margin.left) / slotW)));
  };

  const clearHover = () => {
    if (hoveredIdx >= 0 && hoverBgs[hoveredIdx]) {
      hoverBgs[hoveredIdx].setAttribute('opacity', '0');
    }
    hoveredIdx = -1;
  };

  const renderSingleTooltip = (idx, clientX, clientY) => {
    const item = series[idx];
    if (!item) return;
    let hintText = '';
    if (onSelectRange && onSelectDay) {
      hintText = '拖拽框选时段，或点击查看当日 24 小时明细';
    } else if (onSelectRange) {
      hintText = '按住左键拖拽框选具体时段查看详情';
    } else if (onSelectDay) {
      hintText = '点击柱条查看当日 24 小时明细';
    }
    const drillHint = hintText ? `<div class="tt-hint">${hintText}</div>` : '';
    const speedRow =
      item.avgOutputTokensPerSec > 0
        ? `<div class="tt-row">
             <span class="tt-key">输出速度</span>
             <span class="tt-val">${formatSpeed(item.avgOutputTokensPerSec)}</span>
           </div>`
        : '';
    tooltipEl.innerHTML = `
      <div class="tt-title">${item.label}</div>
      <div class="tt-row">
        <span class="tt-key"><span class="swatch swatch-output"></span>Output</span>
        <span class="tt-val">${formatTokensM(item.outputTokens)}</span>
      </div>
      <div class="tt-row">
        <span class="tt-key"><span class="swatch swatch-input"></span>Input</span>
        <span class="tt-val">${formatTokensM(item.inputTokens)}</span>
      </div>
      <div class="tt-row">
        <span class="tt-key"><span class="swatch swatch-cache"></span>Cache Read</span>
        <span class="tt-val">${formatTokensM(item.cacheReadTokens)}</span>
      </div>
      <div class="tt-divider"></div>
      <div class="tt-row">
        <span class="tt-key">总计</span>
        <span class="tt-val">${formatTokensM(item.totalTokens)}</span>
      </div>
      <div class="tt-row">
        <span class="tt-key">调用 / 命中率</span>
        <span class="tt-val">${item.callCount} 次 · ${formatPct(item.cacheHitRate)}</span>
      </div>
      ${speedRow}
      ${drillHint}
    `;
    tooltipEl.classList.remove('hidden');
    positionTooltip(tooltipEl, clientX, clientY);
  };

  const renderBrushTooltip = (lo, hi, clientX, clientY) => {
    let outTok = 0;
    let inTok = 0;
    let cacheTok = 0;
    let totalTok = 0;
    let calls = 0;
    for (let i = lo; i <= hi; i += 1) {
      const b = series[i];
      outTok += b.outputTokens || 0;
      inTok += b.inputTokens || 0;
      cacheTok += b.cacheReadTokens || 0;
      totalTok += b.totalTokens || 0;
      calls += b.callCount || 0;
    }
    const denom = inTok + cacheTok;
    const hitRate = denom > 0 ? cacheTok / denom : 0;
    const title =
      lo === hi ? series[lo].label : `${series[lo].label} ~ ${series[hi].label}`;
    tooltipEl.innerHTML = `
      <div class="tt-title">框选 ${title} (${hi - lo + 1} 段)</div>
      <div class="tt-row">
        <span class="tt-key"><span class="swatch swatch-output"></span>Output</span>
        <span class="tt-val">${formatTokensM(outTok)}</span>
      </div>
      <div class="tt-row">
        <span class="tt-key"><span class="swatch swatch-input"></span>Input</span>
        <span class="tt-val">${formatTokensM(inTok)}</span>
      </div>
      <div class="tt-row">
        <span class="tt-key"><span class="swatch swatch-cache"></span>Cache Read</span>
        <span class="tt-val">${formatTokensM(cacheTok)}</span>
      </div>
      <div class="tt-divider"></div>
      <div class="tt-row">
        <span class="tt-key">区间总计</span>
        <span class="tt-val">${formatTokensM(totalTok)}</span>
      </div>
      <div class="tt-row">
        <span class="tt-key">调用 / 命中率</span>
        <span class="tt-val">${calls} 次 · ${formatPct(hitRate)}</span>
      </div>
      <div class="tt-hint">松开鼠标查看该时段详情</div>
    `;
    tooltipEl.classList.remove('hidden');
    positionTooltip(tooltipEl, clientX, clientY);
  };

  const onWinMove = (evt) => {
    if (!dragging) return;
    const curIdx = idxFromClientX(evt.clientX);
    dragCurIdx = curIdx;
    if (!isBrushing && (Math.abs(evt.clientX - dragStartX) >= 5 || curIdx !== dragStartIdx)) {
      isBrushing = true;
      clearHover();
    }
    if (isBrushing) {
      const lo = Math.min(dragStartIdx, curIdx);
      const hi = Math.max(dragStartIdx, curIdx);
      brushRect.setAttribute('x', String(margin.left + lo * slotW));
      brushRect.setAttribute('width', String((hi - lo + 1) * slotW));
      brushRect.setAttribute('opacity', '1');
      renderBrushTooltip(lo, hi, evt.clientX, evt.clientY);
    }
  };

  const stopDrag = () => {
    window.removeEventListener('mousemove', onWinMove);
    window.removeEventListener('mouseup', onWinUp);
    dragging = false;
    brushRect.setAttribute('opacity', '0');
  };

  const onWinUp = () => {
    if (!dragging) return;
    const didBrush = isBrushing;
    const startI = dragStartIdx;
    const endI = dragCurIdx;
    stopDrag();
    isBrushing = false;
    tooltipEl.classList.add('hidden');
    clearHover();

    if (didBrush && onSelectRange) {
      const lo = Math.min(startI, endI);
      const hi = Math.max(startI, endI);
      onSelectRange({
        startItem: series[lo],
        endItem: series[hi],
        startIdx: lo,
        endIdx: hi,
        count: hi - lo + 1,
      });
    } else if (!didBrush && onSelectDay && series[startI]) {
      onSelectDay(series[startI].key);
    }
  };

  plotHit.addEventListener('mousemove', (evt) => {
    if (dragging) return;
    const idx = idxFromClientX(evt.clientX);
    if (idx !== hoveredIdx) {
      clearHover();
      hoveredIdx = idx;
      if (hoverBgs[idx]) hoverBgs[idx].setAttribute('opacity', '1');
    }
    renderSingleTooltip(idx, evt.clientX, evt.clientY);
  });

  plotHit.addEventListener('mouseleave', () => {
    if (dragging) return;
    clearHover();
    tooltipEl.classList.add('hidden');
  });

  plotHit.addEventListener('mousedown', (evt) => {
    if (evt.button !== 0) return;
    evt.preventDefault();
    if (!onSelectRange && !onSelectDay) return;
    dragging = true;
    isBrushing = false;
    dragStartIdx = idxFromClientX(evt.clientX);
    dragCurIdx = dragStartIdx;
    dragStartX = evt.clientX;
    window.addEventListener('mousemove', onWinMove);
    window.addEventListener('mouseup', onWinUp);
  });

  container._cleanupBrush = stopDrag;

  overlayGroup.appendChild(plotHit);
  svg.appendChild(barsGroup);
  svg.appendChild(brushGroup);
  svg.appendChild(xAxisGroup);
  svg.appendChild(overlayGroup);
  container.appendChild(svg);
}

export function renderCacheRateChart({
  container,
  emptyEl,
  tooltipEl,
  series,
  averageRate,
  onSelectDay = null,
}) {
  const existingSvg = container.querySelector('svg');
  if (existingSvg) existingSvg.remove();

  const hasActiveBuckets = series.some((d) => d.callCount > 0);
  if (!series.length || !hasActiveBuckets) {
    emptyEl.classList.remove('hidden');
    return;
  }
  emptyEl.classList.add('hidden');

  const rect = container.getBoundingClientRect();
  const width = Math.max(260, Math.floor(rect.width - 32));
  const height = Math.max(150, Math.floor(rect.height - 20));

  const margin = { top: 10, right: 8, bottom: 26, left: 42 };
  const plotW = Math.max(100, width - margin.left - margin.right);
  const plotH = Math.max(70, height - margin.top - margin.bottom);

  const svg = el('svg', {
    class: 'chart-svg',
    viewBox: `0 0 ${width} ${height}`,
    width,
    height,
  });

  // 0%, 50%, 100% gridlines
  const ticks = [1, 0.5, 0];
  for (const t of ticks) {
    const y = Math.round(margin.top + plotH * (1 - t)) + 0.5;
    svg.appendChild(
      el('line', {
        x1: margin.left,
        x2: margin.left + plotW,
        y1: y,
        y2: y,
        stroke: t === 0 ? '#d4d4d8' : '#ececef',
        'stroke-width': 1,
        'stroke-dasharray': t === 0 ? null : '2,2',
      })
    );
    svg.appendChild(
      el(
        'text',
        {
          x: margin.left - 7,
          y: y + 3.5,
          'text-anchor': 'end',
          fill: '#71717a',
          'font-size': 10,
          'font-family': 'var(--font-mono)',
        },
        `${Math.round(t * 100)}%`
      )
    );
  }

  const slotW = plotW / series.length;
  const barW = Math.max(2, Math.min(24, slotW * 0.62));
  const xStep = pickTickStep(series.length, plotW, 64);

  const barsGroup = el('g');
  const overlayGroup = el('g');
  const xAxisGroup = el('g');

  series.forEach((item, idx) => {
    const slotX = margin.left + idx * slotW;
    const barX = slotX + (slotW - barW) / 2;

    const hoverBg = el('rect', {
      x: slotX,
      y: margin.top,
      width: slotW,
      height: plotH,
      rx: 4,
      ry: 4,
      fill: 'rgba(24, 24, 27, 0.045)',
      opacity: 0,
    });
    barsGroup.appendChild(hoverBg);

    if (item.callCount > 0) {
      const rate = Math.max(0, Math.min(1, item.cacheHitRate || 0));
      const barH = Math.max(1, rate * plotH);
      const barRadius = barH >= 4 ? Math.min(3, barW * 0.25) : 0;
      barsGroup.appendChild(
        el('rect', {
          x: barX,
          y: margin.top + plotH - barH,
          width: barW,
          height: barH,
          rx: barRadius || null,
          ry: barRadius || null,
          fill: '#3b6ea5',
          opacity: 0.85,
        })
      );
    }

    if (idx % xStep === 0 || idx === series.length - 1) {
      const isTooCloseToLast =
        idx !== series.length - 1 && (series.length - 1 - idx) * slotW < 44;
      if (!isTooCloseToLast) {
        xAxisGroup.appendChild(
          el(
            'text',
            {
              x: slotX + slotW / 2,
              y: margin.top + plotH + 17,
              'text-anchor': 'middle',
              fill: '#71717a',
              'font-size': 10,
              'font-family': 'var(--font-mono)',
            },
            item.shortLabel
          )
        );
      }
    }

    const hit = el('rect', {
      x: slotX,
      y: margin.top,
      width: slotW,
      height: plotH,
      fill: 'transparent',
      style: onSelectDay ? 'cursor: pointer;' : 'cursor: crosshair;',
    });

    hit.addEventListener('mouseenter', (evt) => {
      hoverBg.setAttribute('opacity', '1');
      const drillHint = onSelectDay
        ? '<div class="tt-hint">点击柱条查看当日 24 小时明细</div>'
        : '';
      tooltipEl.innerHTML = `
        <div class="tt-title">${item.label}</div>
        <div class="tt-row">
          <span class="tt-key">缓存命中率</span>
          <span class="tt-val">${item.callCount > 0 ? formatPct(item.cacheHitRate) : '无调用'}</span>
        </div>
        <div class="tt-row">
          <span class="tt-key">Cache Read</span>
          <span class="tt-val">${formatTokensM(item.cacheReadTokens)}</span>
        </div>
        <div class="tt-row">
          <span class="tt-key">Fresh Input</span>
          <span class="tt-val">${formatTokensM(item.inputTokens)}</span>
        </div>
        <div class="tt-row">
          <span class="tt-key">调用次数</span>
          <span class="tt-val">${item.callCount}</span>
        </div>
        ${drillHint}
      `;
      tooltipEl.classList.remove('hidden');
      positionTooltip(tooltipEl, evt.clientX, evt.clientY);
    });
    hit.addEventListener('mousemove', (evt) => positionTooltip(tooltipEl, evt.clientX, evt.clientY));
    hit.addEventListener('mouseleave', () => {
      hoverBg.setAttribute('opacity', '0');
      tooltipEl.classList.add('hidden');
    });
    if (onSelectDay) {
      hit.addEventListener('click', () => {
        hoverBg.setAttribute('opacity', '0');
        tooltipEl.classList.add('hidden');
        onSelectDay(item.key);
      });
    }

    overlayGroup.appendChild(hit);
  });

  // Average dashed line
  if (averageRate > 0) {
    const avgY = Math.round(margin.top + plotH * (1 - Math.min(1, averageRate))) + 0.5;
    svg.appendChild(
      el('line', {
        x1: margin.left,
        x2: margin.left + plotW,
        y1: avgY,
        y2: avgY,
        stroke: '#18181b',
        'stroke-width': 1,
        'stroke-dasharray': '4,3',
        opacity: 0.75,
      })
    );
  }

  svg.appendChild(barsGroup);
  svg.appendChild(xAxisGroup);
  svg.appendChild(overlayGroup);
  container.appendChild(svg);
}

const HEAT_COLORS = ['#ebedf0', '#9be9a8', '#40c463', '#30a14e', '#216e39'];
const WEEKDAY_NAMES = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];

function pad2(n) {
  return String(n).padStart(2, '0');
}

function formatDayKey(date) {
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
}

function heatLevel(totalTokens, maxTokens) {
  if (!totalTokens || totalTokens <= 0 || maxTokens <= 0) return 0;
  const score = Math.sqrt(totalTokens / maxTokens);
  if (score >= 0.72) return 4;
  if (score >= 0.42) return 3;
  if (score >= 0.16) return 2;
  return 1;
}

export function renderUsageHeatmap({
  container,
  tooltipEl,
  calendar,
  selectedDay,
  onSelectDay,
}) {
  const existingSvg = container.querySelector('svg');
  if (existingSvg) existingSvg.remove();

  const dayMap = new Map();
  let maxTokens = 0;
  for (const item of calendar || []) {
    dayMap.set(item.day, item);
    if (item.totalTokens > maxTokens) maxTokens = item.totalTokens;
  }

  // Build 53-week grid ending on today's local week (Monday = row 0 ... Sunday = row 6)
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const todayKey = formatDayKey(today);
  const todayDow = (today.getDay() + 6) % 7; // Mon=0 .. Sun=6
  const endOfWeek = new Date(today);
  endOfWeek.setDate(today.getDate() + (6 - todayDow));

  const weeks = 53;
  const startDate = new Date(endOfWeek);
  startDate.setDate(endOfWeek.getDate() - weeks * 7 + 1);

  const rect = container.getBoundingClientRect();
  const availW = Math.max(540, Math.floor(rect.width - 36));
  const leftPad = 24;
  const topPad = 18;
  const gap = availW >= 880 ? 4 : 3;
  const rawCell = (availW - leftPad - 2 - (weeks - 1) * gap) / weeks;
  const cell = Math.max(10, Math.min(20, rawCell));
  const cellRadius = Math.min(4, Math.max(2.5, Number((cell * 0.22).toFixed(2))));
  const step = cell + gap;

  const gridW = weeks * step - gap;
  const gridH = 7 * step - gap;
  const width = Math.ceil(leftPad + gridW + 2);
  const height = Math.ceil(topPad + gridH + 4);

  const svg = el('svg', {
    class: 'chart-svg heatmap-svg',
    viewBox: `0 0 ${width} ${height}`,
    width,
    height,
    style: `width: ${width}px; height: ${height}px;`,
  });

  // Weekday labels (一, 三, 五)
  [
    [0, '一'],
    [2, '三'],
    [4, '五'],
  ].forEach(([row, label]) => {
    svg.appendChild(
      el(
        'text',
        {
          x: 0,
          y: topPad + row * step + cell * 0.78,
          fill: '#71717a',
          'font-size': 10,
          'font-family': 'var(--font-sans)',
        },
        label
      )
    );
  });

  const monthsGroup = el('g');
  const cellsGroup = el('g');

  let lastLabeledMonth = -1;
  let lastMonthLabelX = -100;

  for (let w = 0; w < weeks; w += 1) {
    const colX = leftPad + w * step;

    for (let d = 0; d < 7; d += 1) {
      const cur = new Date(startDate);
      cur.setDate(startDate.getDate() + w * 7 + d);
      if (cur > today) continue;

      const dayKey = formatDayKey(cur);
      const month = cur.getMonth();

      if (d === 0 && month !== lastLabeledMonth && colX - lastMonthLabelX >= 32) {
        lastLabeledMonth = month;
        lastMonthLabelX = colX;
        const mLabel = month === 0 ? `${cur.getFullYear()}` : `${month + 1}月`;
        monthsGroup.appendChild(
          el(
            'text',
            {
              x: colX,
              y: 11,
              fill: '#71717a',
              'font-size': 10,
              'font-family': 'var(--font-mono)',
            },
            mLabel
          )
        );
      }

      const info = dayMap.get(dayKey);
      const total = info ? info.totalTokens : 0;
      const lvl = heatLevel(total, maxTokens);
      const isSelected = selectedDay === dayKey;
      const isToday = dayKey === todayKey;

      const square = el('rect', {
        x: colX,
        y: topPad + d * step,
        width: cell,
        height: cell,
        rx: cellRadius,
        ry: cellRadius,
        fill: HEAT_COLORS[lvl],
        stroke: isSelected
          ? '#111113'
          : isToday
            ? '#71717a'
            : 'rgba(27, 31, 35, 0.06)',
        'stroke-width': isSelected ? 1.75 : 1,
        style: 'cursor: pointer;',
      });

      const weekdayName = WEEKDAY_NAMES[d];
      square.addEventListener('mouseenter', (evt) => {
        if (!isSelected) {
          square.setAttribute('stroke', '#111113');
          square.setAttribute('stroke-width', '1.25');
        }
        const hint = isSelected
          ? '<div class="tt-hint">当前已选中 · 再次点击返回多日视图</div>'
          : '<div class="tt-hint">点击查看当日 24 小时明细</div>';
        if (info && info.callCount > 0) {
          tooltipEl.innerHTML = `
            <div class="tt-title">${dayKey} (${weekdayName})</div>
            <div class="tt-row">
              <span class="tt-key">总计</span>
              <span class="tt-val">${formatTokensM(info.totalTokens)}</span>
            </div>
            <div class="tt-row">
              <span class="tt-key">净消耗 (In+Out)</span>
              <span class="tt-val">${formatTokensM(info.netTokens)}</span>
            </div>
            <div class="tt-row">
              <span class="tt-key">Output / Input</span>
              <span class="tt-val">${formatTokensM(info.outputTokens)} / ${formatTokensM(info.inputTokens)}</span>
            </div>
            <div class="tt-row">
              <span class="tt-key">调用次数</span>
              <span class="tt-val">${info.callCount} 次</span>
            </div>
            ${hint}
          `;
        } else {
          tooltipEl.innerHTML = `
            <div class="tt-title">${dayKey} (${weekdayName})</div>
            <div class="tt-row">
              <span class="tt-key">用量</span>
              <span class="tt-val">无记录</span>
            </div>
            ${hint}
          `;
        }
        tooltipEl.classList.remove('hidden');
        positionTooltip(tooltipEl, evt.clientX, evt.clientY);
      });

      square.addEventListener('mousemove', (evt) =>
        positionTooltip(tooltipEl, evt.clientX, evt.clientY)
      );

      square.addEventListener('mouseleave', () => {
        if (!isSelected) {
          square.setAttribute('stroke', isToday ? '#71717a' : 'rgba(27, 31, 35, 0.06)');
          square.setAttribute('stroke-width', '1');
        }
        tooltipEl.classList.add('hidden');
      });

      square.addEventListener('click', () => {
        tooltipEl.classList.add('hidden');
        if (onSelectDay) onSelectDay(dayKey);
      });

      cellsGroup.appendChild(square);
    }
  }

  svg.appendChild(monthsGroup);
  svg.appendChild(cellsGroup);
  container.appendChild(svg);
}
