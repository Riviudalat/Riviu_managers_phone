// One Interaction (Tương tác) campaign: the actors in <config> like/comment/follow the posts in
// <targets.json>. Promoted from target/acceptance-20261010/kit/run-interaction.mjs.
// Order: interaction_parse_links -> (interaction_resolve_links only if a short link is present)
//        -> <out>/intent.json (exclusive, BEFORE any effect) -> interaction_start_thread
//        -> interaction_get every pollSeconds into <out>/ until terminal or maxWatchMinutes.
// Never-replay: an existing intent.json means the campaign may exist; this script then only
// finds it (start.json, or interaction_list by requestId) and watches. It never calls
// interaction_start_thread twice and never calls interaction_retry (see retry-interaction.mjs).
import fs from 'node:fs';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {DEFAULT_PORT, connect, readJson, writeJson, sleep, now, parseArgs, helpAndExit, resolvePath, check} from './ipc.mjs';

const USAGE = `
Tạo đúng một chiến dịch Tương tác rồi theo dõi tới khi kết thúc.
  node scripts/device-acceptance/run-interaction.mjs <targets.json> --config <interaction.json> --out <dir>
       (--publish-dir <dir publish> | --allow-foreign-authors) [--fleet-scope <scope.json>]
       [--exclude <scope-or-config.json,...>] [--port 47182]
  node scripts/device-acceptance/run-interaction.mjs --watch --config <interaction.json> --out <dir>
targets.json = ["https://www.tiktok.com/@x/photo/1", ...]. Mẫu config: examples/interaction.example.json.
--publish-dir: chỉ cho phép bài của tài khoản đã được duyệt ở preflight publish (preflight.json).
Nếu <out>/intent.json đã có thì chỉ tìm và theo dõi chiến dịch, không bao giờ start lần hai.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {flags: ['watch', 'allow-foreign-authors'],
  options: ['config', 'out', 'publish-dir', 'fleet-scope', 'exclude', 'port']});
const cfg = readJson(resolvePath(args.config ?? ''));
check(cfg, 'thiếu --config');
check(args.out, 'thiếu --out (thư mục riêng cho chiến dịch này)');
const OUT = resolvePath(args.out);
const intentFile = path.join(OUT, 'intent.json');
const startFile = path.join(OUT, 'start.json');
const actors = cfg.actorUdids;
check(Array.isArray(actors) && actors.length && new Set(actors).size === actors.length, 'actorUdids phải khác rỗng và không trùng');
const fleet = args['fleet-scope'] ? readJson(resolvePath(args['fleet-scope'])) : null;
if (fleet) check(actors.every(u => fleet.deviceIds.includes(u)), 'có actor nằm ngoài fleet scope');
for (const file of (args.exclude ?? '').split(',').filter(Boolean)) {
  const other = readJson(resolvePath(file));
  const ids = other?.deviceIds ?? other?.udids ?? other?.actorUdids;
  check(Array.isArray(ids), `${file}: không có deviceIds/udids/actorUdids`);
  check(actors.every(u => !ids.includes(u)), `actor trùng máy của ${file}`);
}
for (const text of cfg.manualComments) {
  check(text.trim() && !/[#@]|https?:|www\./i.test(text) && text.trim().split(/\s+/).length <= cfg.maxWords,
    `bình luận tay bị chính sách kit từ chối: ${text}`);
}
check(new Set(cfg.manualComments).size === cfg.manualComments.length && cfg.manualComments.length >= actors.length,
  'manualComments phải không trùng và đủ cho mọi actor');
const maxTargets = cfg.maxTargets ?? 7;
fs.mkdirSync(OUT, {recursive: true});

const c = await connect(args.port ?? DEFAULT_PORT);
try {
  let intent = readJson(intentFile);
  let campaignId = readJson(startFile)?.value?.campaign?.id ?? null;
  if (!intent) {
    check(args._[0] && !args.watch, 'chưa có intent: cần <targets.json> (xem --help)');
    check(args['publish-dir'] || args['allow-foreign-authors'], 'cần --publish-dir (chỉ bài của mình) hoặc --allow-foreign-authors có chủ đích');
    const raw = readJson(resolvePath(args._[0]));
    const urls = Array.isArray(raw) ? raw : raw?.urls;
    check(Array.isArray(urls) && urls.length >= 1 && urls.length <= maxTargets && urls.every(u => typeof u === 'string'),
      `targets: 1-${maxTargets} chuỗi URL`);
    const rawText = urls.join('\n');
    let lines = await c.invoke('interaction_parse_links', {rawText});
    writeJson(path.join(OUT, 'parse.json'), lines);
    if (lines.some(l => l.error)) {
      lines = await c.invoke('interaction_resolve_links', {rawText}); // network only for short links
      writeJson(path.join(OUT, 'resolve.json'), lines);
    }
    const bad = lines.filter(l => l.error || !l.target);
    check(bad.length === 0, `link không dùng được: ${JSON.stringify(bad.map(l => ({line: l.lineNo, error: l.error})))}`);
    const targets = [];
    for (const l of lines) if (!targets.some(t => t.targetKey === l.target.targetKey)) targets.push(l.target);
    if (args['publish-dir'] && !args['allow-foreign-authors']) {
      // Only our own fresh posts: every author must be an account the publish preflight approved.
      const accounts = readJson(path.join(resolvePath(args['publish-dir']), 'preflight.json'))?.approval?.accounts;
      check(accounts, 'publish-dir không có preflight.json với approval.accounts');
      const own = new Set(Object.values(accounts).map(h => String(h).toLowerCase().replace(/^@/, '')));
      const foreign = targets.filter(t => !own.has(t.author.toLowerCase()));
      check(foreign.length === 0, `bài không thuộc tài khoản publish: ${foreign.map(t => t.author).join(',')}`);
    }
    const request = {
      requestId: randomUUID(), targets, actorUdids: actors, messageCount: actors.length,
      instruction: '', maxWords: cfg.maxWords, mode: cfg.mode, shape: cfg.shape,
      actions: cfg.actions, manualComments: cfg.manualComments, mentions: [], mentionParent: false,
      likeParent: false, postDwellSeconds: cfg.postDwellSeconds,
    };
    intent = {request, at: now()};
    writeJson(intentFile, intent, true);
    let start;
    try { start = {ok: true, value: await c.invoke('interaction_start_thread', {request})}; }
    catch (error) { start = {ok: false, ackUnknownNeverReplay: true, error: String(error.message ?? error)}; }
    start.at = now();
    writeJson(startFile, start, true);
    console.log(JSON.stringify({start: start.ok ? 'acknowledged' : 'ackUnknownNeverReplay', campaignId: start.value?.campaign?.id,
      planned: start.value?.campaign?.actionCounters?.planned, error: start.error?.slice(0, 300)}));
    campaignId = start.value?.campaign?.id ?? null;
  }
  if (!campaignId) {
    const list = await c.invoke('interaction_list', {limit: 100});
    campaignId = list.find(row => row.requestId === intent.request.requestId)?.id ?? null;
    writeJson(path.join(OUT, `lookup-${Date.now()}.json`), {requestId: intent.request.requestId, campaignId, at: now()});
    // No row => interaction_start_thread refused before persistence (validation/preflight); nothing ran.
    check(campaignId, 'không có chiến dịch cho intent này (bị từ chối trước khi lưu). Đọc start.json; KHÔNG start lại khi chưa có quyết định mới.');
  }
  const terminal = ['succeeded', 'partial', 'failed', 'cancelled'];
  const deadline = Date.parse(intent.at) + cfg.maxWatchMinutes * 60000;
  let previous = '';
  for (;;) {
    try {
      const detail = await c.invoke('interaction_get', {campaignId});
      writeJson(path.join(OUT, 'detail-latest.json'), detail);
      const s = detail?.summary ?? {};
      const rows = (detail?.assignments ?? []).map(a => `${(a.actorUdid ?? '').slice(-6)}:${a.state}${a.errorCode ? '(' + a.errorCode + ')' : ''}`);
      const line = `${s.state} ${JSON.stringify(s.actionCounters)} ok=${s.succeededMessages} fail=${s.failedMessages}`;
      fs.appendFileSync(path.join(OUT, 'watch.log'), `${now()} ${line} | ${rows.join(' ')}\n`);
      if (line !== previous) { console.log(`${now().slice(11, 19)} ${line}`); previous = line; }
      if (terminal.includes(s.state)) {
        writeJson(path.join(OUT, `final-${campaignId}.json`), detail);
        console.log('TERMINAL ' + s.state);
        break;
      }
    } catch (error) {
      fs.appendFileSync(path.join(OUT, 'watch.log'), `${now()} READ_ERROR ${String(error.message ?? error).slice(0, 200)}\n`);
    }
    if (Date.now() >= deadline) { console.log('WATCH_DEADLINE (chiến dịch vẫn chạy; chạy lại với --watch)'); break; }
    await sleep(cfg.pollSeconds * 1000);
  }
} finally { await c.close(); }
