import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {
  BrainScanner,
  aggregateUsage,
  buildSessionsCsv,
  discoverDefaultBrainSources,
  parseTranscriptText,
  resolveInheritedModels,
} from '../lib/scanner.mjs';
import { createRequestHandler } from '../server.mjs';

async function invokeHandler(handler, urlPath) {
  let status = 200;
  let headers = {};
  const chunks = [];
  const res = {
    writeHead(code, hdrs = {}) {
      status = code;
      headers = { ...hdrs };
    },
    end(chunk) {
      if (chunk !== undefined) {
        chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk), 'utf8'));
      }
    },
  };
  await handler({ url: urlPath }, res);
  const body = Buffer.concat(chunks).toString('utf8');
  return { status, headers, body };
}

const SAMPLE_TRANSCRIPT = [
  JSON.stringify({
    step_index: 0,
    source: 'USER_EXPLICIT',
    type: 'USER_INPUT',
    status: 'DONE',
    created_at: '2026-11-20T10:00:00Z',
    content: '<USER_REQUEST>\n实现一个本地 Token 统计看板\n</USER_REQUEST>',
  }),
  '{invalid json line should be ignored',
  JSON.stringify({
    step_index: 1,
    source: 'MODEL',
    type: 'PLANNER_RESPONSE',
    status: 'DONE',
    created_at: '2026-11-20T10:00:05Z',
    model: 'gemini-4-neon-high',
    input_tokens: 10000,
    cache_read_tokens: 0,
    output_tokens: 500,
    tool_calls: [{ name: 'read_file', args: {} }],
  }),
  JSON.stringify({
    step_index: 2,
    source: 'MODEL',
    type: 'PLANNER_RESPONSE',
    status: 'DONE',
    created_at: '2026-11-20T10:05:00Z',
    model: 'gemini-4-neon-high',
    input_tokens: 2000,
    cache_read_tokens: 10000,
    output_tokens: 800,
    tool_calls: [{ name: 'write_file', args: {} }, { name: 'run_command', args: {} }],
  }),
].join('\n');

test('parseTranscriptText extracts title, tokens, and ignores malformed lines', () => {
  const session = parseTranscriptText(SAMPLE_TRANSCRIPT, 'abc12345-session');
  assert.equal(session.id, 'abc12345-session');
  assert.equal(session.title, '实现一个本地 Token 统计看板');
  assert.equal(session.events.length, 2);
  assert.equal(session.events[0].inputTokens, 10000);
  assert.equal(session.events[0].outputTokens, 500);
  assert.equal(session.events[1].cacheReadTokens, 10000);
  assert.equal(session.events[1].toolCount, 2);
});

test('BrainScanner scans directories, caches by mtime, and aggregates accurately', async () => {
  const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'agy-usage-test-'));
  try {
    const s1Logs = path.join(tmpDir, 'sess-1', '.system_generated', 'logs');
    const s2Logs = path.join(tmpDir, 'sess-2', '.system_generated', 'logs');
    await fs.mkdir(s1Logs, { recursive: true });
    await fs.mkdir(s2Logs, { recursive: true });

    await fs.writeFile(path.join(s1Logs, 'transcript.jsonl'), SAMPLE_TRANSCRIPT, 'utf8');
    await fs.writeFile(
      path.join(s2Logs, 'transcript_full.jsonl'),
      JSON.stringify({
        step_index: 1,
        type: 'PLANNER_RESPONSE',
        status: 'DONE',
        created_at: '2026-11-18T08:00:00Z',
        model: 'gemini-flash',
        input_tokens: 4000,
        cache_read_tokens: 6000,
        output_tokens: 200,
      }),
      'utf8'
    );

    const scanner = new BrainScanner(tmpDir);
    const scan1 = await scanner.scan();
    assert.equal(scan1.error, null);
    assert.equal(scan1.parsedSessions, 2);

    const now = Date.parse('2026-11-20T12:00:00Z');
    const agg7d = aggregateUsage(scan1, { range: '7d', granularity: 'day', now });
    assert.equal(agg7d.summary.sessionCount, 2);
    assert.equal(agg7d.summary.callCount, 3);
    assert.equal(agg7d.summary.inputTokens, 16000);
    assert.equal(agg7d.summary.outputTokens, 1500);
    assert.equal(agg7d.summary.cacheReadTokens, 16000);
    assert.equal(agg7d.summary.totalTokens, 33500);
    assert.equal(agg7d.summary.cacheHitRate, 0.5);
    assert.equal(agg7d.models.length, 2);
    assert.equal(agg7d.models[0].model, 'gemini-4-neon-high');
    assert.ok(agg7d.series.length >= 7);

    // 24h range excludes sess-2 from Nov 18
    const agg24h = aggregateUsage(scan1, { range: '24h', granularity: 'hour', now });
    assert.equal(agg24h.summary.sessionCount, 1);
    assert.equal(agg24h.summary.callCount, 2);
    assert.ok(agg24h.series.length >= 24);

    // Single-day hourly drill-down for 2026-11-18
    const aggDay = aggregateUsage(scan1, { day: '2026-11-18', now });
    assert.equal(aggDay.meta.range, 'day');
    assert.equal(aggDay.meta.granularity, 'hour');
    assert.equal(aggDay.meta.selectedDay, '2026-11-18');
    assert.equal(aggDay.series.length, 24);
    assert.equal(aggDay.summary.sessionCount, 1);
    assert.equal(aggDay.summary.totalTokens, 10200);
    assert.ok(aggDay.meta.availableDays.includes('2026-11-18'));
    assert.ok(aggDay.meta.availableDays.includes('2026-11-20'));

    // Model filter
    const aggFlash = aggregateUsage(scan1, { range: '7d', model: 'gemini-flash', now });
    assert.equal(aggFlash.summary.sessionCount, 1);
    assert.equal(aggFlash.summary.totalTokens, 10200);

    // CSV export
    const csv = buildSessionsCsv(agg7d.sessions);
    assert.ok(csv.startsWith('\uFEFFsession_id,'));
    assert.ok(csv.includes('实现一个本地 Token 统计看板'));

    // HTTP handler routes smoke test (in-process)
    const handler = createRequestHandler({ brainDir: tmpDir });
    const healthRes = await invokeHandler(handler, '/api/health');
    assert.equal(healthRes.status, 200);
    assert.equal(JSON.parse(healthRes.body).ok, true);

    const usageRes = await invokeHandler(handler, '/api/usage?range=all');
    assert.equal(usageRes.status, 200);
    const usage = JSON.parse(usageRes.body);
    assert.equal(usage.summary.callCount, 3);

    const detailRes = await invokeHandler(handler, '/api/sessions/sess-1');
    assert.equal(detailRes.status, 200);
    const detail = JSON.parse(detailRes.body);
    assert.equal(detail.events.length, 2);

    const missingRes = await invokeHandler(handler, '/api/sessions/not-found');
    assert.equal(missingRes.status, 404);

    const csvRes = await invokeHandler(handler, '/api/export.csv?range=all');
    assert.equal(csvRes.status, 200);
    assert.ok(csvRes.body.includes('sess-1'));

    const htmlRes = await invokeHandler(handler, '/');
    assert.equal(htmlRes.status, 200);
    assert.ok(htmlRes.body.includes('Antigravity'));

    const cssRes = await invokeHandler(handler, '/styles.css');
    assert.equal(cssRes.status, 200);
    assert.ok(cssRes.headers['Content-Type'].includes('text/css'));
  } finally {
    await fs.rm(tmpDir, { recursive: true, force: true });
  }
});

test('recovers legacy missing model fields from USER_SETTINGS_CHANGE and parent subagent links', () => {
  const parentText = [
    JSON.stringify({
      step_index: 0,
      type: 'USER_INPUT',
      created_at: '2026-11-18T02:00:00Z',
      content:
        '<USER_REQUEST>run task</USER_REQUEST><USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Orchestrator (Carbon).</USER_SETTINGS_CHANGE>',
    }),
    JSON.stringify({
      step_index: 1,
      type: 'PLANNER_RESPONSE',
      created_at: '2026-11-18T02:00:05Z',
      input_tokens: 5000,
      output_tokens: 300,
      tool_calls: [
        {
          name: 'invoke_subagent',
          args: { Model: '"inherit"', TypeName: '"research"', Role: '"Code Auditor"' },
        },
      ],
    }),
    JSON.stringify({
      step_index: 2,
      type: 'GENERIC',
      created_at: '2026-11-18T02:00:06Z',
      content: 'Created subagent: {"conversationId": "child-1111-aaaa"}',
    }),
  ].join('\n');

  const childText = JSON.stringify({
    step_index: 1,
    type: 'PLANNER_RESPONSE',
    created_at: '2026-11-18T02:00:10Z',
    input_tokens: 8000,
    output_tokens: 400,
  });

  const parent = parseTranscriptText(parentText, 'parent-0000');
  const child = parseTranscriptText(childText, 'child-1111-aaaa');
  assert.equal(parent.events[0].model, 'Orchestrator (Carbon)');
  assert.equal(child.events[0].model, 'unknown');

  const resolved = resolveInheritedModels([parent, child]);
  const resolvedChild = resolved.find((s) => s.id === 'child-1111-aaaa');
  assert.equal(resolvedChild.events[0].model, 'Orchestrator (Carbon)');
  assert.equal(resolvedChild.isSubagent, true);
  assert.equal(resolvedChild.rootParentId, 'parent-0000');
  assert.equal(resolvedChild.parentTitle, 'run task');
  assert.equal(resolvedChild.title, 'Code Auditor');
  assert.equal(resolvedChild.subagentType, 'research');
  assert.equal(parent.events[0].genDurationMs, 1000);
  assert.equal(parent.events[0].tokensPerSec, 300);
});

test('computes multi-tool output speed (tok/s) and ignores user wait gaps', () => {
  const txt = [
    JSON.stringify({
      step_index: 0,
      type: 'PLANNER_RESPONSE',
      created_at: '2026-11-24T10:00:00Z',
      model: 'gemini-4-neon-high',
      input_tokens: 2000,
      output_tokens: 1000,
      tool_calls: [{ name: 'write_file' }, { name: 'write_file' }],
    }),
    JSON.stringify({
      step_index: 1,
      type: 'GENERIC',
      created_at: '2026-11-24T10:00:04Z',
      content: 'ok',
    }),
    JSON.stringify({
      step_index: 2,
      type: 'GENERIC',
      created_at: '2026-11-24T10:00:10Z',
      content: 'ok',
    }),
    JSON.stringify({
      step_index: 3,
      type: 'PLANNER_RESPONSE',
      created_at: '2026-11-24T10:00:11Z',
      model: 'gemini-4-neon-high',
      input_tokens: 2500,
      output_tokens: 200,
    }),
    JSON.stringify({
      step_index: 4,
      type: 'USER_INPUT',
      created_at: '2026-11-24T10:15:00Z',
      content: 'next',
    }),
  ].join('\n');

  const parsed = parseTranscriptText(txt, 'speed-session');
  assert.equal(parsed.events.length, 2);
  assert.equal(parsed.events[0].genDurationMs, 10000);
  assert.equal(parsed.events[0].tokensPerSec, 100);
  assert.equal(parsed.events[1].genDurationMs, null);
  assert.equal(parsed.events[1].tokensPerSec, null);

  const agg = aggregateUsage(
    {
      brainDir: 'test',
      scannedAt: Date.now(),
      error: null,
      totalDirs: 1,
      parsedSessions: 1,
      skippedDirs: 0,
      sessions: [parsed],
    },
    { range: 'all' }
  );
  assert.equal(agg.summary.avgOutputTokensPerSec, 100);
  assert.equal(agg.summary.peakOutputTokensPerSec, 100);
  assert.equal(agg.summary.speedSampleCount, 1);
  assert.equal(agg.models[0].avgOutputTokensPerSec, 100);
  assert.equal(agg.sessions[0].avgOutputTokensPerSec, 100);
});

test('strips T3 Code bootstrap wrappers and aggregates/filters multi-source brain dirs', async () => {
  const t3Text = [
    JSON.stringify({
      step_index: 0,
      type: 'USER_INPUT',
      created_at: '2026-11-24T11:08:22Z',
      content:
        '<USER_REQUEST>\n<t3_code_instructions>\n## T3 Code interaction mode\n</t3_code_instructions>\n<runtime_info>runtime details</runtime_info>\n</USER_REQUEST>\n<ADDITIONAL_METADATA>meta</ADDITIONAL_METADATA>',
    }),
    JSON.stringify({
      step_index: 1,
      type: 'PLANNER_RESPONSE',
      created_at: '2026-11-24T11:08:22Z',
      model: 'gemini-pro-orchestrator-neon',
      input_tokens: 8000,
      output_tokens: 60,
      cache_read_tokens: 0,
    }),
    JSON.stringify({
      step_index: 2,
      type: 'USER_INPUT',
      created_at: '2026-11-24T11:08:42Z',
      content:
        '<USER_REQUEST>\n写一个 hello world<runtime_info>In case you are asked</runtime_info>\n</USER_REQUEST>',
    }),
    JSON.stringify({
      step_index: 3,
      type: 'PLANNER_RESPONSE',
      created_at: '2026-11-24T11:08:45Z',
      model: 'gemini-4-hydrogen-medium',
      input_tokens: 3400,
      output_tokens: 200,
      cache_read_tokens: 1000,
    }),
  ].join('\n');

  const parsedT3 = parseTranscriptText(t3Text, 't3-sess-1');
  assert.equal(parsedT3.hasUserInput, true);
  assert.equal(parsedT3.title, '写一个 hello world');

  const tmpHome = await fs.mkdtemp(path.join(os.tmpdir(), 'agy-multi-src-'));
  try {
    const appDir = path.join(tmpHome, '.gemini', 'antigravity', 'brain');
    const cliDir = path.join(tmpHome, '.gemini', 'antigravity-cli', 'brain');
    const t3Dir = path.join(
      tmpHome,
      '.t3',
      'userdata',
      'providers',
      'antigravity',
      'inst-1',
      'antigravity-acp',
      'brain'
    );
    await fs.mkdir(path.join(appDir, 'shared-id', '.system_generated', 'logs'), { recursive: true });
    await fs.mkdir(path.join(cliDir, 'cli-1', '.system_generated', 'logs'), { recursive: true });
    await fs.mkdir(path.join(t3Dir, 'shared-id', '.system_generated', 'logs'), { recursive: true });

    await fs.writeFile(
      path.join(appDir, 'shared-id', '.system_generated', 'logs', 'transcript.jsonl'),
      JSON.stringify({
        step_index: 0,
        type: 'USER_INPUT',
        created_at: '2026-11-24T10:00:00Z',
        content: '<USER_REQUEST>App 会话</USER_REQUEST>',
      }) +
        '\n' +
        JSON.stringify({
          step_index: 1,
          type: 'PLANNER_RESPONSE',
          created_at: '2026-11-24T10:00:05Z',
          model: 'gemini-4-neon-high',
          input_tokens: 1000,
          output_tokens: 100,
        }),
      'utf8'
    );
    await fs.writeFile(
      path.join(cliDir, 'cli-1', '.system_generated', 'logs', 'transcript.jsonl'),
      JSON.stringify({
        step_index: 0,
        type: 'USER_INPUT',
        created_at: '2026-11-24T11:00:00Z',
        content: '<USER_REQUEST>CLI 会话</USER_REQUEST>',
      }) +
        '\n' +
        JSON.stringify({
          step_index: 1,
          type: 'PLANNER_RESPONSE',
          created_at: '2026-11-24T11:00:05Z',
          model: 'gemini-4-neon-high',
          input_tokens: 2000,
          output_tokens: 200,
        }),
      'utf8'
    );
    await fs.writeFile(
      path.join(t3Dir, 'shared-id', '.system_generated', 'logs', 'transcript.jsonl'),
      t3Text,
      'utf8'
    );

    const sources = await discoverDefaultBrainSources(tmpHome);
    assert.equal(sources.length, 3);
    assert.deepEqual(
      sources.map((s) => s.id),
      ['app', 'cli', 't3']
    );

    const handler = createRequestHandler({ sources });
    const allRes = await invokeHandler(handler, '/api/usage?range=all');
    assert.equal(allRes.status, 200);
    const allData = JSON.parse(allRes.body);
    assert.equal(allData.summary.sessionCount, 3);
    assert.equal(allData.meta.sources.length, 3);
    assert.equal(allData.meta.sourceFilter, 'all');

    const t3Res = await invokeHandler(handler, '/api/usage?range=all&source=t3');
    const t3Data = JSON.parse(t3Res.body);
    assert.equal(t3Data.summary.sessionCount, 1);
    assert.equal(t3Data.summary.totalTokens, 12660);
    assert.equal(t3Data.meta.sourceFilter, 't3');
    assert.equal(t3Data.sessions[0].source, 't3');
    assert.equal(t3Data.sessions[0].sourceLabel, 'T3 Code');
    assert.equal(t3Data.meta.sources.length, 3);

    const detailT3 = await invokeHandler(handler, '/api/sessions/shared-id?source=t3');
    assert.equal(detailT3.status, 200);
    const detailT3Json = JSON.parse(detailT3.body);
    assert.equal(detailT3Json.source, 't3');
    assert.equal(detailT3Json.title, '写一个 hello world');

    const csv = buildSessionsCsv(t3Data.sessions);
    assert.ok(csv.startsWith('\uFEFFsession_id,source,'));
    assert.ok(csv.includes('T3 Code'));

    const filteredSources = await discoverDefaultBrainSources(tmpHome, {
      includeApp: true,
      includeCli: false,
      includeT3: false,
    });
    assert.equal(filteredSources.length, 1);
    assert.equal(filteredSources[0].label, 'Antigravity');

    const customRangeRes = await invokeHandler(
      handler,
      '/api/usage?range=custom&startDay=2026-11-23&endDay=2026-11-25'
    );
    assert.equal(customRangeRes.status, 200);
    const customRangeData = JSON.parse(customRangeRes.body);
    assert.equal(customRangeData.meta.range, 'custom');
    assert.equal(customRangeData.meta.selectedStartDay, '2026-11-23');
    assert.equal(customRangeData.meta.selectedEndDay, '2026-11-25');
    assert.equal(customRangeData.series.length, 3);
    assert.equal(customRangeData.summary.sessionCount, 3);
  } finally {
    await fs.rm(tmpHome, { recursive: true, force: true });
  }
});
