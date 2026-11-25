import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';

const TRANSCRIPT_CANDIDATES = [
  path.join('.system_generated', 'logs', 'transcript.jsonl'),
  path.join('.system_generated', 'logs', 'transcript_full.jsonl'),
];

export function getDefaultBrainDir() {
  if (process.env.ANTIGRAVITY_BRAIN_DIR) {
    return path.resolve(process.env.ANTIGRAVITY_BRAIN_DIR);
  }
  return path.join(os.homedir(), '.gemini', 'antigravity', 'brain');
}

export function inferSourceFromDir(dir) {
  const norm = String(dir || '')
    .replace(/\\/g, '/')
    .replace(/\/+$/, '')
    .toLowerCase();
  if (norm.includes('/antigravity-acp/') || norm.endsWith('/antigravity-acp/brain')) {
    return { id: 't3', label: 'T3 Code' };
  }
  if (norm.includes('/antigravity-cli/') || norm.endsWith('/antigravity-cli/brain')) {
    return { id: 'cli', label: 'CLI' };
  }
  if (norm.includes('/.gemini/antigravity/') || norm.endsWith('/antigravity/brain')) {
    return { id: 'app', label: 'Antigravity' };
  }
  return { id: 'custom', label: '自定义目录' };
}

async function fileExists(p) {
  try {
    const stat = await fs.stat(p);
    return stat.isFile();
  } catch {
    return false;
  }
}

async function pathExists(p) {
  try {
    await fs.stat(p);
    return true;
  } catch {
    return false;
  }
}

export async function isAntigravityInstalled() {
  const localApp = process.env.LOCALAPPDATA || '';
  const pf = process.env.ProgramFiles || '';
  const candidates = [];
  if (localApp) {
    candidates.push(path.join(localApp, 'Programs', 'antigravity', 'Antigravity.exe'));
    candidates.push(path.join(localApp, 'Programs', 'Antigravity', 'Antigravity.exe'));
  }
  if (pf) {
    candidates.push(path.join(pf, 'Antigravity', 'Antigravity.exe'));
  }
  if (process.platform === 'darwin') {
    candidates.push('/Applications/Antigravity.app');
  }
  for (const c of candidates) {
    if (await pathExists(c)) return true;
  }
  return false;
}

export async function isT3CodeInstalled() {
  const localApp = process.env.LOCALAPPDATA || '';
  const pf = process.env.ProgramFiles || '';
  const candidates = [];
  if (localApp) {
    candidates.push(path.join(localApp, 'Programs', 't3code', 'T3 Code (Beta).exe'));
    candidates.push(path.join(localApp, 'Programs', 't3code', 'T3 Code.exe'));
  }
  if (pf) {
    candidates.push(path.join(pf, 'T3 Code', 'T3 Code.exe'));
    candidates.push(path.join(pf, 'T3 Code', 'T3 Code (Beta).exe'));
  }
  for (const c of candidates) {
    if (await fileExists(c)) return true;
  }
  return false;
}

export async function isAgyCliInstalled() {
  const localApp = process.env.LOCALAPPDATA || '';
  const home = os.homedir();
  const shimDir = localApp
    ? path.join(localApp, 'AgyDock', 'cli-shim').toLowerCase().replace(/[\\/]+$/, '')
    : '';

  const pathEnv = process.env.PATH || process.env.Path || '';
  for (const dir of pathEnv.split(path.delimiter).filter(Boolean)) {
    const normDir = dir.trim().toLowerCase().replace(/[\\/]+$/, '');
    if (shimDir && normDir === shimDir) continue;
    if (
      (await fileExists(path.join(dir, 'agy.exe'))) ||
      (await fileExists(path.join(dir, 'antigravity-cli.exe'))) ||
      (await fileExists(path.join(dir, 'agy')))
    ) {
      return true;
    }
  }

  const fixed = [
    path.join(home, '.local', 'bin', 'agy.exe'),
    path.join(home, '.local', 'bin', 'agy'),
    path.join(home, '.antigravity', 'bin', 'agy.exe'),
    path.join(home, '.antigravity', 'bin', 'agy'),
  ];
  if (localApp) {
    fixed.push(path.join(localApp, 'Programs', 'antigravity-cli', 'agy.exe'));
    fixed.push(path.join(localApp, 'antigravity', 'agy.exe'));
  }
  for (const c of fixed) {
    if (await fileExists(c)) return true;
  }
  return false;
}

export async function discoverDefaultBrainSources(homeDir = os.homedir(), options = {}) {
  let includeApp = options.includeApp ?? true;
  let includeCli = options.includeCli ?? true;
  let includeT3 = options.includeT3 ?? true;

  if (options.checkInstalled) {
    const [appOk, cliOk, t3Ok] = await Promise.all([
      isAntigravityInstalled(),
      isAgyCliInstalled(),
      isT3CodeInstalled(),
    ]);
    includeApp = appOk;
    includeCli = cliOk;
    includeT3 = t3Ok;
    if (!includeApp && !includeCli && !includeT3) {
      const fallbackDir = path.join(homeDir, '.gemini', 'antigravity', 'brain');
      if (await pathExists(fallbackDir)) {
        includeApp = true;
      }
    }
  }

  const sources = [];
  if (includeApp) {
    sources.push({
      id: 'app',
      label: 'Antigravity',
      dir: path.join(homeDir, '.gemini', 'antigravity', 'brain'),
    });
  }
  if (includeCli) {
    sources.push({
      id: 'cli',
      label: 'CLI',
      dir: path.join(homeDir, '.gemini', 'antigravity-cli', 'brain'),
    });
  }

  if (includeT3) {
    const t3Root = path.join(homeDir, '.t3', 'userdata', 'providers', 'antigravity');
    try {
      const entries = await fs.readdir(t3Root, { withFileTypes: true });
      const subdirs = entries
        .filter((e) => e.isDirectory())
        .map((e) => e.name)
        .sort();
      for (const sub of subdirs) {
        const acpBrain = path.join(t3Root, sub, 'antigravity-acp', 'brain');
        try {
          const stat = await fs.stat(acpBrain);
          if (stat.isDirectory()) {
            sources.push({
              id: 't3',
              label: 'T3 Code',
              dir: acpBrain,
            });
          }
        } catch {
          // ignore missing subdir
        }
      }
    } catch {
      // ignore missing ~/.t3
    }
  }

  return sources;
}

function extractUserTitle(content) {
  if (typeof content !== 'string' || !content.trim()) return '';
  const match = content.match(/<USER_REQUEST>([\s\S]*?)<\/USER_REQUEST>/i);
  let text = match ? match[1] : content;
  text = text
    .replace(
      /<t3_code_instructions>[\s\S]*?(?:<\/t3_code_instructions>|$)|<runtime_info>[\s\S]*?(?:<\/runtime_info>|$)/gi,
      ' '
    )
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (text.length > 84) {
    return `${text.slice(0, 81)}...`;
  }
  return text;
}

function toNumber(value) {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

const SETTING_MODEL_MAP = {
  'Gemini 4 Carbon (High)': 'gemini-4-carbon-high',
  'Gemini 4 Neon (High)': 'gemini-4-neon-high',
  'Gemini 4 Hydrogen (High)': 'gemini-4-hydrogen-tiered-high',
};

function normalizeSettingModel(raw) {
  const cleaned = String(raw || '').trim().replace(/\.$/, '');
  if (!cleaned || cleaned.toLowerCase() === 'none') return null;
  return SETTING_MODEL_MAP[cleaned] || cleaned;
}

function unquoteArg(val) {
  if (typeof val !== 'string') return null;
  let s = val.trim();
  if ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'"))) {
    s = s.slice(1, -1).trim();
  }
  return s || null;
}

function extractSystemPromptPreview(content) {
  if (typeof content !== 'string' || !content.trim()) return '';
  const m = content.match(/content=([\s\S]*?)(?:<\/SYSTEM_MESSAGE>|$)/i);
  let text = (m ? m[1] : content)
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (text.length > 84) {
    return `${text.slice(0, 81)}...`;
  }
  return text;
}

function finalizePendingSpeed(pending, events) {
  if (!pending || pending.lastToolTs === null) return;
  const durationMs = pending.lastToolTs - pending.startTs;
  if (durationMs < 1000 || durationMs > 600_000 || pending.outputTokens <= 0) return;
  const tps = pending.outputTokens / (durationMs / 1000);
  if (!Number.isFinite(tps) || tps < 1 || tps > 1200) return;
  const ev = events[pending.eventIdx];
  if (ev) {
    ev.genDurationMs = durationMs;
    ev.tokensPerSec = Math.round(tps * 10) / 10;
  }
}

export function parseTranscriptText(text, sessionId) {
  const lines = text.split(/\r?\n/);
  let title = '';
  let hasUserInput = false;
  let systemSenderId = null;
  let systemPromptPreview = '';
  const events = [];
  const spawnedChildren = [];
  let firstTs = null;
  let lastTs = null;
  let currentSettingModel = null;
  let lastExplicitModel = null;
  const pendingSubagentCalls = [];
  let pendingSpeed = null;

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line || line[0] !== '{') continue;

    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue;
    }

    const ts = entry.created_at ? Date.parse(entry.created_at) : NaN;
    if (Number.isFinite(ts)) {
      if (firstTs === null || ts < firstTs) firstTs = ts;
      if (lastTs === null || ts > lastTs) lastTs = ts;
    }

    if (pendingSpeed) {
      if (
        entry.type === 'USER_INPUT' ||
        entry.type === 'SYSTEM_MESSAGE' ||
        entry.type === 'PLANNER_RESPONSE'
      ) {
        finalizePendingSpeed(pendingSpeed, events);
        pendingSpeed = null;
      } else {
        if (Number.isFinite(ts)) {
          if (pendingSpeed.lastToolTs === null || ts > pendingSpeed.lastToolTs) {
            pendingSpeed.lastToolTs = ts;
          }
        }
        pendingSpeed.remainingTools = Math.max(0, pendingSpeed.remainingTools - 1);
        if (pendingSpeed.remainingTools === 0) {
          finalizePendingSpeed(pendingSpeed, events);
          pendingSpeed = null;
        }
      }
    }

    if (entry.type === 'USER_INPUT') {
      hasUserInput = true;
    }

    if (typeof entry.content === 'string') {
      if (!title && entry.type === 'USER_INPUT') {
        title = extractUserTitle(entry.content);
      }

      if (!systemSenderId && entry.type === 'SYSTEM_MESSAGE') {
        const sm = entry.content.match(/sender=([a-z0-9_-]{8,})/i);
        if (sm && sm[1] !== 'system' && sm[1] !== sessionId) {
          systemSenderId = sm[1];
        }
        if (!systemPromptPreview) {
          systemPromptPreview = extractSystemPromptPreview(entry.content);
        }
      }

      const settingMatches = entry.content.matchAll(
        /changed setting `Model Selection` from .*? to ([^.\r\n]+)/g
      );
      for (const m of settingMatches) {
        const normalized = normalizeSettingModel(m[1]);
        if (normalized) currentSettingModel = normalized;
      }

      const cidMatches = entry.content.matchAll(/"conversationId"\s*:\s*"([a-z0-9_-]+)"/gi);
      for (const cm of cidMatches) {
        const childId = cm[1];
        if (childId && childId !== sessionId) {
          const matchedCall = pendingSubagentCalls.shift() || null;
          spawnedChildren.push({
            childId,
            parentId: sessionId,
            parentModel: lastExplicitModel || currentSettingModel,
            requestedModel: matchedCall?.requestedModel || null,
            role: matchedCall?.role || null,
            type: matchedCall?.type || null,
          });
        }
      }

      // Capture role/type metadata from manage_subagents listings
      const listMatches = entry.content.matchAll(
        /\{"role":"([^"]+)","type":"([^"]+)","conversationId":"([a-z0-9_-]+)"/gi
      );
      for (const lm of listMatches) {
        const childId = lm[3];
        if (childId && childId !== sessionId) {
          spawnedChildren.push({
            childId,
            parentId: sessionId,
            parentModel: lastExplicitModel || currentSettingModel,
            requestedModel: null,
            role: lm[1],
            type: lm[2],
          });
        }
      }
    }

    const usage = entry.usage && typeof entry.usage === 'object' ? entry.usage : {};
    const inputTokens = toNumber(
      entry.input_tokens ?? entry.prompt_tokens ?? usage.input_tokens ?? usage.prompt_tokens
    );
    const outputTokens = toNumber(
      entry.output_tokens ?? entry.completion_tokens ?? usage.output_tokens ?? usage.completion_tokens
    );
    const cacheReadTokens = toNumber(
      entry.cache_read_tokens ??
        entry.cached_tokens ??
        entry.cache_read_input_tokens ??
        usage.cache_read_tokens ??
        usage.cached_tokens
    );
    const cacheWriteTokens = toNumber(
      entry.cache_creation_tokens ?? entry.cache_write_tokens ?? usage.cache_creation_tokens
    );

    const toolCalls = Array.isArray(entry.tool_calls) ? entry.tool_calls : [];
    let hasAskQuestion = false;
    for (const tc of toolCalls) {
      if (tc && tc.name === 'ask_question') {
        hasAskQuestion = true;
      }
      if (tc && tc.name === 'invoke_subagent') {
        pendingSubagentCalls.push({
          requestedModel: unquoteArg(tc.args?.Model),
          role: unquoteArg(tc.args?.Role),
          type: unquoteArg(tc.args?.TypeName),
        });
      }
    }

    const hasTokens = inputTokens > 0 || outputTokens > 0 || cacheReadTokens > 0 || cacheWriteTokens > 0;
    if (!hasTokens && entry.type !== 'PLANNER_RESPONSE') {
      continue;
    }

    const rawModel = entry.model ?? entry.model_name ?? entry.model_id ?? usage.model;
    const explicitModel = typeof rawModel === 'string' && rawModel.trim() ? rawModel.trim() : null;
    if (explicitModel) {
      lastExplicitModel = explicitModel;
    }

    const resolvedLocalModel = explicitModel || currentSettingModel || null;
    const totalTokens = inputTokens + outputTokens + cacheReadTokens + cacheWriteTokens;

    events.push({
      stepIndex: Number.isFinite(Number(entry.step_index)) ? Number(entry.step_index) : events.length,
      ts: Number.isFinite(ts) ? ts : lastTs ?? Date.now(),
      rawModel: resolvedLocalModel,
      model: resolvedLocalModel || 'unknown',
      status: typeof entry.status === 'string' ? entry.status : 'DONE',
      inputTokens,
      outputTokens,
      cacheReadTokens,
      cacheWriteTokens,
      totalTokens,
      toolCount: toolCalls.length,
      genDurationMs: null,
      tokensPerSec: null,
    });

    if (
      entry.type === 'PLANNER_RESPONSE' &&
      toolCalls.length > 0 &&
      outputTokens > 0 &&
      !hasAskQuestion &&
      Number.isFinite(ts)
    ) {
      pendingSpeed = {
        eventIdx: events.length - 1,
        startTs: ts,
        outputTokens,
        remainingTools: toolCalls.length,
        lastToolTs: null,
      };
    }
  }

  finalizePendingSpeed(pendingSpeed, events);

  return {
    id: sessionId,
    source: 'app',
    sourceLabel: 'Antigravity',
    title: title || systemPromptPreview || `Session ${sessionId.slice(0, 8)}`,
    hasUserInput,
    systemSenderId,
    systemPromptPreview,
    startedAt: firstTs,
    lastAt: lastTs,
    settingModel: currentSettingModel || lastExplicitModel,
    spawnedChildren,
    events,
  };
}

export function resolveInheritedModels(sessions) {
  const sessionById = new Map();
  const childToHint = new Map();

  for (const s of sessions) {
    sessionById.set(s.id, s);
  }

  for (const s of sessions) {
    for (const child of s.spawnedChildren || []) {
      const existing = childToHint.get(child.childId);
      if (!existing) {
        childToHint.set(child.childId, { ...child });
      } else {
        if (!existing.role && child.role) existing.role = child.role;
        if (!existing.type && child.type) existing.type = child.type;
        if (!existing.requestedModel && child.requestedModel) {
          existing.requestedModel = child.requestedModel;
        }
        if (!existing.parentModel && child.parentModel) existing.parentModel = child.parentModel;
      }
    }
  }

  // Fallback link from child SYSTEM_MESSAGE sender=<parentId>
  for (const s of sessions) {
    if (!s.hasUserInput && s.systemSenderId && !childToHint.has(s.id)) {
      const parent = sessionById.get(s.systemSenderId);
      childToHint.set(s.id, {
        childId: s.id,
        parentId: s.systemSenderId,
        parentModel: parent?.settingModel || null,
        requestedModel: null,
        role: null,
        type: null,
      });
    }
  }

  const resolveRootParentId = (sessionId, depth = 0) => {
    if (depth > 8) return null;
    const hint = childToHint.get(sessionId);
    if (!hint || !hint.parentId) return null;
    const parent = sessionById.get(hint.parentId);
    if (parent && parent.hasUserInput) return parent.id;
    const ancestor = resolveRootParentId(hint.parentId, depth + 1);
    return ancestor || hint.parentId;
  };

  const resolveForSession = (sessionId, depth = 0) => {
    if (depth > 6) return null;
    const s = sessionById.get(sessionId);
    if (s?.settingModel) return s.settingModel;

    const hint = childToHint.get(sessionId);
    if (!hint) return null;

    if (hint.requestedModel && hint.requestedModel !== 'inherit') {
      return hint.requestedModel;
    }
    if (hint.parentModel) return hint.parentModel;
    return resolveForSession(hint.parentId, depth + 1);
  };

  return sessions.map((s) => {
    const hint = childToHint.get(s.id) || null;
    const isSubagent = !s.hasUserInput && Boolean(hint);
    const parentId = isSubagent ? hint.parentId : null;
    const rootParentId = isSubagent ? resolveRootParentId(s.id) : null;
    const rootParent = rootParentId ? sessionById.get(rootParentId) : null;
    const subagentRole = isSubagent ? hint.role || null : null;
    const subagentType = isSubagent ? hint.type || null : null;

    let displayTitle = s.title;
    if (isSubagent) {
      if (subagentRole) {
        displayTitle = subagentRole;
      } else if (s.systemPromptPreview) {
        displayTitle = s.systemPromptPreview;
      }
    }

    const needsModelResolution = s.events.some((ev) => !ev.rawModel);
    const inheritedModel = needsModelResolution ? resolveForSession(s.id) : null;

    return {
      ...s,
      title: displayTitle,
      isSubagent,
      parentId,
      rootParentId,
      parentTitle: rootParent ? rootParent.title : null,
      subagentRole,
      subagentType,
      events: inheritedModel
        ? s.events.map((ev) => (ev.rawModel ? ev : { ...ev, model: inheritedModel }))
        : s.events,
    };
  });
}

function canonicalSourceOrder(id) {
  switch (id) {
    case 'app':
      return 0;
    case 'cli':
      return 1;
    case 't3':
      return 2;
    case 'custom':
      return 3;
    default:
      return 4;
  }
}

function pathDedupKey(p) {
  return path
    .resolve(p)
    .replace(/\\/g, '/')
    .replace(/\/+$/, '')
    .toLowerCase();
}

export class BrainScanner {
  constructor(input) {
    if (typeof input === 'string' && input.trim()) {
      this.mode = 'custom';
      this.customDir = path.resolve(input);
      this.explicitSources = null;
      this.brainDir = this.customDir;
    } else if (Array.isArray(input)) {
      this.mode = 'explicit';
      this.customDir = null;
      this.explicitSources = input.map((s) => ({
        id: s.id || 'custom',
        label: s.label || s.id || '自定义目录',
        dir: path.resolve(s.dir),
      }));
      this.brainDir = this.explicitSources[0]?.dir || getDefaultBrainDir();
    } else {
      this.mode = 'auto';
      this.customDir = null;
      this.explicitSources = null;
      this.brainDir = getDefaultBrainDir();
    }
    this.cache = new Map();
  }

  async scan() {
    const startedAt = Date.now();
    const isCustomDir = this.mode === 'custom';
    let rawSpecs;
    if (this.mode === 'custom') {
      const inferred = inferSourceFromDir(this.customDir);
      rawSpecs = [{ id: inferred.id, label: inferred.label, dir: this.customDir }];
    } else if (this.mode === 'explicit') {
      rawSpecs = this.explicitSources || [];
    } else {
      rawSpecs = await discoverDefaultBrainSources(os.homedir(), { checkInstalled: true });
    }

    const seenDirs = new Set();
    const specs = [];
    for (const spec of rawSpecs) {
      const key = pathDedupKey(spec.dir);
      if (key && !seenDirs.has(key)) {
        seenDirs.add(key);
        specs.push({ ...spec, dir: path.resolve(spec.dir) });
      }
    }

    const sourceMap = new Map();
    const activePaths = new Set();
    const allSessions = [];
    let totalDirs = 0;
    let skippedDirs = 0;
    const existingDirs = [];

    for (const spec of specs) {
      let info = sourceMap.get(spec.id);
      if (!info) {
        info = {
          id: spec.id,
          label: spec.label,
          dirs: [],
          exists: false,
          sessionCount: 0,
        };
        sourceMap.set(spec.id, info);
      }
      if (!info.dirs.includes(spec.dir)) {
        info.dirs.push(spec.dir);
      }

      let entries;
      try {
        entries = await fs.readdir(spec.dir, { withFileTypes: true });
      } catch (err) {
        if (isCustomDir) {
          const sources = Array.from(sourceMap.values()).sort(
            (a, b) => canonicalSourceOrder(a.id) - canonicalSourceOrder(b.id)
          );
          return {
            brainDir: spec.dir,
            isCustomDir: true,
            sources,
            scannedAt: startedAt,
            error:
              err.code === 'ENOENT' ? `目录不存在: ${spec.dir}` : `无法读取目录: ${err.message}`,
            totalDirs: 0,
            parsedSessions: 0,
            skippedDirs: 0,
            sessions: [],
          };
        }
        if (err.code === 'EACCES' || err.code === 'EPERM') {
          skippedDirs += 1;
        }
        continue;
      }

      info.exists = true;
      existingDirs.push(spec.dir);
      const groupSessions = [];

      for (const dirent of entries) {
        if (!dirent.isDirectory()) continue;
        totalDirs += 1;

        const sessionId = dirent.name;
        const sessionDir = path.join(spec.dir, sessionId);
        let filePath = null;
        let stat = null;
        let permissionDenied = false;

        for (const rel of TRANSCRIPT_CANDIDATES) {
          const candidate = path.join(sessionDir, rel);
          try {
            const s = await fs.stat(candidate);
            if (s.isFile()) {
              filePath = candidate;
              stat = s;
              break;
            }
          } catch (err) {
            if (err.code === 'EACCES' || err.code === 'EPERM') {
              permissionDenied = true;
              break;
            }
          }
        }

        if (permissionDenied) {
          skippedDirs += 1;
          continue;
        }

        if (!filePath || !stat) {
          continue;
        }

        activePaths.add(filePath);
        const cached = this.cache.get(filePath);
        if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size) {
          groupSessions.push({
            ...cached.session,
            source: spec.id,
            sourceLabel: spec.label,
          });
          continue;
        }

        try {
          const text = await fs.readFile(filePath, 'utf8');
          const session = parseTranscriptText(text, sessionId);
          session.source = spec.id;
          session.sourceLabel = spec.label;
          if (session.startedAt === null) session.startedAt = stat.birthtimeMs || stat.mtimeMs;
          if (session.lastAt === null) session.lastAt = stat.mtimeMs;
          this.cache.set(filePath, {
            mtimeMs: stat.mtimeMs,
            size: stat.size,
            session,
          });
          groupSessions.push(session);
        } catch (err) {
          if (err.code === 'EACCES' || err.code === 'EPERM') {
            skippedDirs += 1;
          }
        }
      }

      const resolvedGroup = resolveInheritedModels(groupSessions).map((s) => ({
        ...s,
        source: spec.id,
        sourceLabel: spec.label,
      }));
      info.sessionCount += resolvedGroup.length;
      allSessions.push(...resolvedGroup);
    }

    for (const key of this.cache.keys()) {
      if (!activePaths.has(key)) {
        this.cache.delete(key);
      }
    }

    const sources = Array.from(sourceMap.values()).sort(
      (a, b) => canonicalSourceOrder(a.id) - canonicalSourceOrder(b.id)
    );
    const brainDirDisplay = existingDirs.length
      ? existingDirs.join(' ; ')
      : this.customDir || specs[0]?.dir || getDefaultBrainDir();
    this.brainDir = brainDirDisplay;

    if (!isCustomDir && existingDirs.length === 0) {
      return {
        brainDir: brainDirDisplay,
        isCustomDir,
        sources,
        scannedAt: Date.now(),
        error: '未找到任何可用的 Antigravity / CLI 日志目录',
        totalDirs: 0,
        parsedSessions: 0,
        skippedDirs,
        sessions: [],
      };
    }

    return {
      brainDir: brainDirDisplay,
      isCustomDir,
      sources,
      scannedAt: Date.now(),
      error: null,
      totalDirs,
      parsedSessions: allSessions.length,
      skippedDirs,
      sessions: allSessions,
    };
  }
}

function pad2(n) {
  return String(n).padStart(2, '0');
}

function bucketStart(ts, granularity) {
  const d = new Date(ts);
  if (granularity === 'hour') {
    d.setMinutes(0, 0, 0);
  } else {
    d.setHours(0, 0, 0, 0);
  }
  return d.getTime();
}

function formatBucketLabels(ms, granularity) {
  const d = new Date(ms);
  const y = d.getFullYear();
  const m = pad2(d.getMonth() + 1);
  const day = pad2(d.getDate());
  const hh = pad2(d.getHours());

  if (granularity === 'hour') {
    return {
      key: `${y}-${m}-${day}T${hh}`,
      label: `${m}-${day} ${hh}:00`,
      shortLabel: `${hh}:00`,
    };
  }
  return {
    key: `${y}-${m}-${day}`,
    label: `${y}-${m}-${day}`,
    shortLabel: `${m}/${day}`,
  };
}

function calcCacheHitRate(inputTokens, cacheReadTokens) {
  const denom = inputTokens + cacheReadTokens;
  if (denom <= 0) return 0;
  return cacheReadTokens / denom;
}

function parseLocalDayBounds(dayStr) {
  if (typeof dayStr !== 'string') return null;
  const m = dayStr.trim().match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  const start = new Date(year, month - 1, day, 0, 0, 0, 0);
  if (
    start.getFullYear() !== year ||
    start.getMonth() !== month - 1 ||
    start.getDate() !== day
  ) {
    return null;
  }
  const end = new Date(year, month - 1, day + 1, 0, 0, 0, 0);
  return {
    dayKey: `${m[1]}-${m[2]}-${m[3]}`,
    startMs: start.getTime(),
    endMs: end.getTime(),
  };
}

function formatLocalDayKey(ts) {
  const d = new Date(ts);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

function parseLocalDayRange(startDayStr, endDayStr) {
  const s = parseLocalDayBounds(startDayStr);
  const e = parseLocalDayBounds(endDayStr);
  if (!s || !e) return null;
  if (s.startMs <= e.startMs) {
    return {
      startDay: s.dayKey,
      endDay: e.dayKey,
      startMs: s.startMs,
      endMs: e.endMs,
    };
  }
  return {
    startDay: e.dayKey,
    endDay: s.dayKey,
    startMs: e.startMs,
    endMs: s.endMs,
  };
}

export function aggregateUsage(scanResult, options = {}) {
  const now = Number.isFinite(options.now) ? options.now : Date.now();
  const tsWindow =
    Number.isFinite(options.startTs) &&
    Number.isFinite(options.endTs) &&
    options.endTs > options.startTs
      ? { startMs: options.startTs, endMs: options.endTs }
      : null;
  const dayRangeBounds = !tsWindow ? parseLocalDayRange(options.startDay, options.endDay) : null;
  const dayBounds = !tsWindow && !dayRangeBounds ? parseLocalDayBounds(options.day) : null;

  const range =
    tsWindow || dayRangeBounds
      ? 'custom'
      : dayBounds
        ? 'day'
        : ['24h', '7d', '30d', 'all'].includes(options.range)
          ? options.range
          : '7d';

  const granularity = tsWindow
    ? options.granularity === 'day'
      ? 'day'
      : 'hour'
    : dayRangeBounds
      ? options.granularity === 'hour' || options.granularity === 'day'
        ? options.granularity
        : dayRangeBounds.startDay === dayRangeBounds.endDay
          ? 'hour'
          : 'day'
      : dayBounds
        ? 'hour'
        : options.granularity === 'hour' || options.granularity === 'day'
          ? options.granularity
          : range === '24h'
            ? 'hour'
            : 'day';
  const modelFilter = options.model && options.model !== 'all' ? options.model : null;
  const sourceFilter = options.source && options.source !== 'all' ? options.source : null;

  let rangeStart = null;
  let rangeEnd = null;
  if (tsWindow) {
    rangeStart = tsWindow.startMs;
    rangeEnd = tsWindow.endMs;
  } else if (dayRangeBounds) {
    rangeStart = dayRangeBounds.startMs;
    rangeEnd = dayRangeBounds.endMs;
  } else if (dayBounds) {
    rangeStart = dayBounds.startMs;
    rangeEnd = dayBounds.endMs;
  } else if (range === '24h') rangeStart = now - 24 * 60 * 60 * 1000;
  else if (range === '7d') rangeStart = now - 7 * 24 * 60 * 60 * 1000;
  else if (range === '30d') rangeStart = now - 30 * 24 * 60 * 60 * 1000;

  const allModelsSet = new Set();
  const availableDaysSet = new Set();
  const calendarMap = new Map();
  let allTimeEventsCount = 0;

  const summary = {
    totalTokens: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    netTokens: 0,
    cacheHitRate: 0,
    callCount: 0,
    sessionCount: 0,
    userSessionCount: 0,
    subagentSessionCount: 0,
    toolCallCount: 0,
    avgTokensPerCall: 0,
    avgTokensPerSession: 0,
    avgOutputTokensPerSec: 0,
    peakOutputTokensPerSec: 0,
    speedSampleCount: 0,
    lastActivityAt: null,
  };

  let summarySpeedOut = 0;
  let summarySpeedMs = 0;
  let summaryPeakStrict = 0;
  let summaryPeakAny = 0;

  const calcWeightedTps = (out, durMs) =>
    durMs > 0 && out > 0 ? Math.round(((out * 1000) / durMs) * 10) / 10 : 0;

  const bucketMap = new Map();
  const modelMap = new Map();
  const sessionSummaries = [];
  let minEventTs = null;
  let maxEventTs = null;

  for (const session of scanResult.sessions) {
    if (sourceFilter && (session.source || 'app') !== sourceFilter) continue;
    let sessInput = 0;
    let sessOutput = 0;
    let sessCacheRead = 0;
    let sessCacheWrite = 0;
    let sessCalls = 0;
    let sessTools = 0;
    let sessSpeedOut = 0;
    let sessSpeedMs = 0;
    let sessSpeedSamples = 0;
    let sessFirstTs = null;
    let sessLastTs = null;
    const sessModelCounts = new Map();

    for (const ev of session.events) {
      allModelsSet.add(ev.model);
      const evDayKey = formatLocalDayKey(ev.ts);
      availableDaysSet.add(evDayKey);
      allTimeEventsCount += 1;

      if (!modelFilter || ev.model === modelFilter) {
        let cal = calendarMap.get(evDayKey);
        if (!cal) {
          cal = {
            day: evDayKey,
            inputTokens: 0,
            outputTokens: 0,
            cacheReadTokens: 0,
            netTokens: 0,
            totalTokens: 0,
            callCount: 0,
          };
          calendarMap.set(evDayKey, cal);
        }
        cal.inputTokens += ev.inputTokens;
        cal.outputTokens += ev.outputTokens;
        cal.cacheReadTokens += ev.cacheReadTokens;
        cal.netTokens += ev.inputTokens + ev.outputTokens;
        cal.totalTokens += ev.totalTokens;
        cal.callCount += 1;
      }

      if (rangeStart !== null && ev.ts < rangeStart) continue;
      if (rangeEnd !== null && ev.ts >= rangeEnd) continue;
      if (modelFilter && ev.model !== modelFilter) continue;

      sessCalls += 1;
      sessInput += ev.inputTokens;
      sessOutput += ev.outputTokens;
      sessCacheRead += ev.cacheReadTokens;
      sessCacheWrite += ev.cacheWriteTokens;
      sessTools += ev.toolCount;

      if (sessFirstTs === null || ev.ts < sessFirstTs) sessFirstTs = ev.ts;
      if (sessLastTs === null || ev.ts > sessLastTs) sessLastTs = ev.ts;
      if (minEventTs === null || ev.ts < minEventTs) minEventTs = ev.ts;
      if (maxEventTs === null || ev.ts > maxEventTs) maxEventTs = ev.ts;

      sessModelCounts.set(ev.model, (sessModelCounts.get(ev.model) || 0) + ev.totalTokens);

      const bMs = bucketStart(ev.ts, granularity);
      let bucket = bucketMap.get(bMs);
      if (!bucket) {
        const labels = formatBucketLabels(bMs, granularity);
        bucket = {
          ts: bMs,
          ...labels,
          inputTokens: 0,
          outputTokens: 0,
          cacheReadTokens: 0,
          cacheWriteTokens: 0,
          totalTokens: 0,
          netTokens: 0,
          callCount: 0,
          cacheHitRate: 0,
          avgOutputTokensPerSec: 0,
          speedSampleCount: 0,
          _speedOut: 0,
          _speedMs: 0,
        };
        bucketMap.set(bMs, bucket);
      }
      bucket.inputTokens += ev.inputTokens;
      bucket.outputTokens += ev.outputTokens;
      bucket.cacheReadTokens += ev.cacheReadTokens;
      bucket.cacheWriteTokens += ev.cacheWriteTokens;
      bucket.totalTokens += ev.totalTokens;
      bucket.netTokens += ev.inputTokens + ev.outputTokens;
      bucket.callCount += 1;

      let mAgg = modelMap.get(ev.model);
      if (!mAgg) {
        mAgg = {
          model: ev.model,
          callCount: 0,
          sessions: new Set(),
          inputTokens: 0,
          outputTokens: 0,
          cacheReadTokens: 0,
          cacheWriteTokens: 0,
          totalTokens: 0,
          netTokens: 0,
          speedOutputTokens: 0,
          speedDurationMs: 0,
          speedSampleCount: 0,
          peakStrict: 0,
          peakAny: 0,
        };
        modelMap.set(ev.model, mAgg);
      }
      mAgg.callCount += 1;
      mAgg.sessions.add(`${session.source || 'app'}:${session.id}`);
      mAgg.inputTokens += ev.inputTokens;
      mAgg.outputTokens += ev.outputTokens;
      mAgg.cacheReadTokens += ev.cacheReadTokens;
      mAgg.cacheWriteTokens += ev.cacheWriteTokens;
      mAgg.totalTokens += ev.totalTokens;
      mAgg.netTokens += ev.inputTokens + ev.outputTokens;

      if (ev.genDurationMs > 0 && ev.tokensPerSec > 0) {
        sessSpeedOut += ev.outputTokens;
        sessSpeedMs += ev.genDurationMs;
        sessSpeedSamples += 1;

        summarySpeedOut += ev.outputTokens;
        summarySpeedMs += ev.genDurationMs;
        summary.speedSampleCount += 1;
        if (ev.tokensPerSec > summaryPeakAny) summaryPeakAny = ev.tokensPerSec;
        if (ev.genDurationMs >= 3000 && ev.outputTokens >= 100 && ev.tokensPerSec > summaryPeakStrict) {
          summaryPeakStrict = ev.tokensPerSec;
        }

        bucket._speedOut += ev.outputTokens;
        bucket._speedMs += ev.genDurationMs;
        bucket.speedSampleCount += 1;

        mAgg.speedOutputTokens += ev.outputTokens;
        mAgg.speedDurationMs += ev.genDurationMs;
        mAgg.speedSampleCount += 1;
        if (ev.tokensPerSec > mAgg.peakAny) mAgg.peakAny = ev.tokensPerSec;
        if (ev.genDurationMs >= 3000 && ev.outputTokens >= 100 && ev.tokensPerSec > mAgg.peakStrict) {
          mAgg.peakStrict = ev.tokensPerSec;
        }
      }
    }

    if (sessCalls > 0) {
      const sessTotal = sessInput + sessOutput + sessCacheRead + sessCacheWrite;
      const sessNet = sessInput + sessOutput;
      let primaryModel = 'unknown';
      let bestModelTokens = -1;
      for (const [mName, mTok] of sessModelCounts.entries()) {
        if (mTok > bestModelTokens) {
          bestModelTokens = mTok;
          primaryModel = mName;
        }
      }

      summary.totalTokens += sessTotal;
      summary.inputTokens += sessInput;
      summary.outputTokens += sessOutput;
      summary.cacheReadTokens += sessCacheRead;
      summary.cacheWriteTokens += sessCacheWrite;
      summary.netTokens += sessNet;
      summary.callCount += sessCalls;
      summary.sessionCount += 1;
      if (session.isSubagent) {
        summary.subagentSessionCount += 1;
      } else {
        summary.userSessionCount += 1;
      }
      summary.toolCallCount += sessTools;
      if (summary.lastActivityAt === null || sessLastTs > summary.lastActivityAt) {
        summary.lastActivityAt = sessLastTs;
      }

      sessionSummaries.push({
        id: session.id,
        source: session.source || 'app',
        sourceLabel: session.sourceLabel || 'Antigravity',
        title: session.title,
        isSubagent: Boolean(session.isSubagent),
        parentId: session.parentId || null,
        rootParentId: session.rootParentId || null,
        parentTitle: session.parentTitle || null,
        subagentRole: session.subagentRole || null,
        subagentType: session.subagentType || null,
        startedAt: sessFirstTs,
        lastAt: sessLastTs,
        durationMs: Math.max(0, (sessLastTs ?? 0) - (sessFirstTs ?? 0)),
        primaryModel,
        models: Array.from(sessModelCounts.keys()),
        callCount: sessCalls,
        toolCallCount: sessTools,
        inputTokens: sessInput,
        outputTokens: sessOutput,
        cacheReadTokens: sessCacheRead,
        cacheWriteTokens: sessCacheWrite,
        netTokens: sessNet,
        totalTokens: sessTotal,
        cacheHitRate: calcCacheHitRate(sessInput, sessCacheRead),
        avgOutputTokensPerSec: calcWeightedTps(sessSpeedOut, sessSpeedMs),
        speedOutputTokens: sessSpeedOut,
        speedDurationMs: sessSpeedMs,
        speedSampleCount: sessSpeedSamples,
      });
    }
  }

  summary.cacheHitRate = calcCacheHitRate(summary.inputTokens, summary.cacheReadTokens);
  summary.avgOutputTokensPerSec = calcWeightedTps(summarySpeedOut, summarySpeedMs);
  summary.peakOutputTokensPerSec = summaryPeakStrict > 0 ? summaryPeakStrict : summaryPeakAny;
  summary.avgTokensPerCall = summary.callCount > 0 ? Math.round(summary.totalTokens / summary.callCount) : 0;
  summary.avgTokensPerSession =
    summary.sessionCount > 0 ? Math.round(summary.totalTokens / summary.sessionCount) : 0;

  // Build continuous timeline series
  const series = [];
  const stepMs = granularity === 'hour' ? 60 * 60 * 1000 : 24 * 60 * 60 * 1000;
  let timelineStart;
  let timelineEnd = bucketStart(now, granularity);

  if (tsWindow) {
    timelineStart = bucketStart(tsWindow.startMs, granularity);
    timelineEnd = bucketStart(Math.max(tsWindow.startMs, tsWindow.endMs - 1), granularity);
  } else if (dayRangeBounds) {
    timelineStart = bucketStart(dayRangeBounds.startMs, granularity);
    timelineEnd = bucketStart(Math.max(dayRangeBounds.startMs, dayRangeBounds.endMs - 1), granularity);
    if (granularity === 'hour') {
      const minAllowed = timelineEnd - 167 * 60 * 60 * 1000;
      if (timelineStart < minAllowed) timelineStart = minAllowed;
    }
  } else if (dayBounds) {
    timelineStart = dayBounds.startMs;
    const lastHourDate = new Date(dayBounds.startMs);
    lastHourDate.setHours(23, 0, 0, 0);
    timelineEnd = lastHourDate.getTime();
  } else if (rangeStart !== null) {
    timelineStart = bucketStart(rangeStart, granularity);
    if (granularity === 'hour' && range === '30d') {
      // Cap hourly view on 30d to last 72h to keep chart readable
      timelineStart = bucketStart(now - 72 * 60 * 60 * 1000, 'hour');
    } else if (granularity === 'hour' && range === '7d') {
      timelineStart = bucketStart(now - 7 * 24 * 60 * 60 * 1000, 'hour');
    }
  } else if (minEventTs !== null) {
    timelineStart = bucketStart(minEventTs, granularity);
    if (maxEventTs !== null && bucketStart(maxEventTs, granularity) > timelineEnd) {
      timelineEnd = bucketStart(maxEventTs, granularity);
    }
    if (granularity === 'hour') {
      const maxSpanStart = timelineEnd - 72 * 60 * 60 * 1000;
      if (timelineStart < maxSpanStart) timelineStart = maxSpanStart;
    }
  } else {
    timelineStart = timelineEnd - (granularity === 'hour' ? 23 : 6) * stepMs;
  }

  // Guard against excessive bucket count
  const maxBuckets = granularity === 'hour' ? 168 : 365;
  const cursorDate = new Date(timelineStart);
  let count = 0;
  while (cursorDate.getTime() <= timelineEnd && count < maxBuckets) {
    const ms = cursorDate.getTime();
    const existing = bucketMap.get(ms);
    if (existing) {
      existing.cacheHitRate = calcCacheHitRate(existing.inputTokens, existing.cacheReadTokens);
      existing.avgOutputTokensPerSec = calcWeightedTps(existing._speedOut, existing._speedMs);
      const { _speedOut, _speedMs, ...cleanBucket } = existing;
      series.push(cleanBucket);
    } else {
      const labels = formatBucketLabels(ms, granularity);
      series.push({
        ts: ms,
        ...labels,
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        totalTokens: 0,
        netTokens: 0,
        callCount: 0,
        cacheHitRate: 0,
        avgOutputTokensPerSec: 0,
        speedSampleCount: 0,
      });
    }
    if (granularity === 'hour') {
      cursorDate.setHours(cursorDate.getHours() + 1);
    } else {
      cursorDate.setDate(cursorDate.getDate() + 1);
    }
    count += 1;
  }

  const models = Array.from(modelMap.values())
    .map((m) => ({
      model: m.model,
      callCount: m.callCount,
      sessionCount: m.sessions.size,
      inputTokens: m.inputTokens,
      outputTokens: m.outputTokens,
      cacheReadTokens: m.cacheReadTokens,
      cacheWriteTokens: m.cacheWriteTokens,
      totalTokens: m.totalTokens,
      netTokens: m.netTokens,
      cacheHitRate: calcCacheHitRate(m.inputTokens, m.cacheReadTokens),
      avgOutputTokensPerSec: calcWeightedTps(m.speedOutputTokens, m.speedDurationMs),
      peakOutputTokensPerSec: m.peakStrict > 0 ? m.peakStrict : m.peakAny,
      speedSampleCount: m.speedSampleCount,
      share: summary.totalTokens > 0 ? m.totalTokens / summary.totalTokens : 0,
    }))
    .sort((a, b) => b.totalTokens - a.totalTokens);

  sessionSummaries.sort((a, b) => (b.lastAt ?? 0) - (a.lastAt ?? 0));

  const calendar = Array.from(calendarMap.values()).sort((a, b) =>
    a.day.localeCompare(b.day)
  );

  return {
    meta: {
      brainDir: scanResult.brainDir,
      isCustomDir: Boolean(scanResult.isCustomDir),
      sources: scanResult.sources || [],
      sourceFilter: sourceFilter || 'all',
      scannedAt: scanResult.scannedAt,
      error: scanResult.error,
      totalDirs: scanResult.totalDirs,
      parsedSessions: scanResult.parsedSessions,
      skippedDirs: scanResult.skippedDirs,
      allTimeEventsCount,
      availableModels: Array.from(allModelsSet).sort(),
      availableDays: Array.from(availableDaysSet).sort(),
      selectedDay: dayBounds ? dayBounds.dayKey : null,
      selectedStartDay: dayRangeBounds ? dayRangeBounds.startDay : null,
      selectedEndDay: dayRangeBounds ? dayRangeBounds.endDay : null,
      selectedStartTs: tsWindow ? tsWindow.startMs : null,
      selectedEndTs: tsWindow ? tsWindow.endMs : null,
      range,
      granularity,
      modelFilter: modelFilter || 'all',
    },
    summary,
    series,
    calendar,
    models,
    sessions: sessionSummaries,
  };
}

export function buildSessionsCsv(sessions) {
  const header = [
    'session_id',
    'source',
    'kind',
    'parent_id',
    'title',
    'started_at',
    'last_at',
    'primary_model',
    'calls',
    'tool_calls',
    'input_tokens',
    'output_tokens',
    'cache_read_tokens',
    'net_tokens',
    'total_tokens',
    'cache_hit_rate',
    'avg_output_tps',
  ];

  const escapeCell = (value) => {
    const str = String(value ?? '');
    if (/[",\r\n]/.test(str)) {
      return `"${str.replace(/"/g, '""')}"`;
    }
    return str;
  };

  const rows = sessions.map((s) => [
    s.id,
    s.sourceLabel || s.source || '',
    s.isSubagent ? 'subagent' : 'main',
    s.rootParentId || s.parentId || '',
    s.title,
    s.startedAt ? new Date(s.startedAt).toISOString() : '',
    s.lastAt ? new Date(s.lastAt).toISOString() : '',
    s.primaryModel,
    s.callCount,
    s.toolCallCount,
    s.inputTokens,
    s.outputTokens,
    s.cacheReadTokens,
    s.netTokens,
    s.totalTokens,
    `${(s.cacheHitRate * 100).toFixed(2)}%`,
    s.avgOutputTokensPerSec > 0 ? s.avgOutputTokensPerSec.toFixed(1) : '',
  ]);

  return '\uFEFF' + [header, ...rows].map((row) => row.map(escapeCell).join(',')).join('\r\n');
}
