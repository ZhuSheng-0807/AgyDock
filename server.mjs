import http from 'node:http';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  BrainScanner,
  aggregateUsage,
  buildSessionsCsv,
  getDefaultBrainDir,
} from './lib/scanner.mjs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const PUBLIC_DIR = path.join(__dirname, 'public');

const MIME_TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
};

function parseCliArgs(argv) {
  const args = {
    port: Number(process.env.PORT) || 3789,
    host: process.env.HOST || '127.0.0.1',
    dir: getDefaultBrainDir(),
    hasExplicitDir: Boolean(process.env.ANTIGRAVITY_BRAIN_DIR?.trim()),
  };

  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if ((token === '--port' || token === '-p') && argv[i + 1]) {
      args.port = Number(argv[i + 1]) || args.port;
      i += 1;
    } else if ((token === '--dir' || token === '-d') && argv[i + 1]) {
      args.dir = path.resolve(argv[i + 1]);
      args.hasExplicitDir = true;
      i += 1;
    } else if (token === '--host' && argv[i + 1]) {
      args.host = argv[i + 1];
      i += 1;
    }
  }
  return args;
}

function sendJson(res, statusCode, payload) {
  const body = JSON.stringify(payload);
  res.writeHead(statusCode, {
    'Content-Type': 'application/json; charset=utf-8',
    'Cache-Control': 'no-store',
  });
  res.end(body);
}

async function serveStatic(res, pathname) {
  const normalized = pathname === '/' ? '/index.html' : pathname;
  const safePath = path.normalize(normalized).replace(/^(\.\.(\/|\\|$))+/, '');
  const fullPath = path.join(PUBLIC_DIR, safePath);

  if (!fullPath.startsWith(PUBLIC_DIR)) {
    res.writeHead(403);
    res.end('Forbidden');
    return;
  }

  try {
    const content = await fs.readFile(fullPath);
    const ext = path.extname(fullPath).toLowerCase();
    res.writeHead(200, {
      'Content-Type': MIME_TYPES[ext] || 'application/octet-stream',
      'Cache-Control': 'no-cache',
    });
    res.end(content);
  } catch (err) {
    if (err.code === 'ENOENT') {
      res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      res.end('Not Found');
    } else {
      res.writeHead(500, { 'Content-Type': 'text/plain; charset=utf-8' });
      res.end('Internal Server Error');
    }
  }
}

export function createRequestHandler({ brainDir, sources, autoDiscover } = {}) {
  const scannerInput = Array.isArray(sources)
    ? sources
    : autoDiscover
      ? { autoDiscover: true }
      : brainDir || { autoDiscover: true };
  const scanner = new BrainScanner(scannerInput);

  return async function handleRequest(req, res) {
    const url = new URL(req.url || '/', 'http://127.0.0.1');
    const pathname = url.pathname;

    try {
      if (pathname === '/api/health') {
        sendJson(res, 200, { ok: true, brainDir: scanner.brainDir });
        return;
      }

      if (pathname === '/api/usage') {
        const scanResult = await scanner.scan();
        const startTsRaw = url.searchParams.get('startTs');
        const endTsRaw = url.searchParams.get('endTs');
        const data = aggregateUsage(scanResult, {
          range: url.searchParams.get('range') || '7d',
          granularity: url.searchParams.get('granularity') || undefined,
          model: url.searchParams.get('model') || 'all',
          source: url.searchParams.get('source') || 'all',
          day: url.searchParams.get('day') || undefined,
          startDay: url.searchParams.get('startDay') || undefined,
          endDay: url.searchParams.get('endDay') || undefined,
          startTs: startTsRaw !== null ? Number(startTsRaw) : undefined,
          endTs: endTsRaw !== null ? Number(endTsRaw) : undefined,
        });
        sendJson(res, 200, data);
        return;
      }

      if (pathname.startsWith('/api/sessions/')) {
        const sessionId = decodeURIComponent(pathname.slice('/api/sessions/'.length));
        const sourceFilter = url.searchParams.get('source');
        const scanResult = await scanner.scan();
        const session = scanResult.sessions.find(
          (s) =>
            s.id === sessionId &&
            (!sourceFilter || sourceFilter === 'all' || (s.source || 'app') === sourceFilter)
        );
        if (!session) {
          sendJson(res, 404, { error: 'Session not found' });
          return;
        }
        sendJson(res, 200, {
          id: session.id,
          source: session.source || 'app',
          sourceLabel: session.sourceLabel || 'Antigravity',
          title: session.title,
          startedAt: session.startedAt,
          lastAt: session.lastAt,
          events: session.events,
        });
        return;
      }

      if (pathname === '/api/export.csv') {
        const scanResult = await scanner.scan();
        const startTsRaw = url.searchParams.get('startTs');
        const endTsRaw = url.searchParams.get('endTs');
        const data = aggregateUsage(scanResult, {
          range: url.searchParams.get('range') || '7d',
          granularity: url.searchParams.get('granularity') || undefined,
          model: url.searchParams.get('model') || 'all',
          source: url.searchParams.get('source') || 'all',
          day: url.searchParams.get('day') || undefined,
          startDay: url.searchParams.get('startDay') || undefined,
          endDay: url.searchParams.get('endDay') || undefined,
          startTs: startTsRaw !== null ? Number(startTsRaw) : undefined,
          endTs: endTsRaw !== null ? Number(endTsRaw) : undefined,
        });
        const csv = buildSessionsCsv(data.sessions);
        const suffix =
          data.meta.selectedStartDay && data.meta.selectedEndDay
            ? `${data.meta.selectedStartDay}_to_${data.meta.selectedEndDay}`
            : data.meta.selectedDay || data.meta.range;
        const filename = `antigravity-usage-${suffix}.csv`;
        res.writeHead(200, {
          'Content-Type': 'text/csv; charset=utf-8',
          'Content-Disposition': `attachment; filename="${filename}"`,
          'Cache-Control': 'no-store',
        });
        res.end(csv);
        return;
      }

      await serveStatic(res, pathname);
    } catch (err) {
      sendJson(res, 500, { error: err.message || 'Server error' });
    }
  };
}

export function createAppServer(options) {
  return http.createServer(createRequestHandler(options));
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  const { port, host, dir, hasExplicitDir } = parseCliArgs(process.argv.slice(2));
  const server = createAppServer(hasExplicitDir ? { brainDir: dir } : { autoDiscover: true });

  server.listen(port, host, () => {
    console.log(`Antigravity Usage Dashboard listening at http://${host}:${port}`);
    console.log(
      hasExplicitDir
        ? `Scanning brain directory: ${dir}`
        : 'Scanning installed brain directories (Antigravity / CLI / T3 Code)'
    );
  });
}
