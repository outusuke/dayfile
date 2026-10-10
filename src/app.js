(() => {
'use strict';

const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const $ = (id) => document.getElementById(id);
const msgOf = (e) => (typeof e === 'string' ? e : (e && e.message) || String(e));

// local time throughout; toISOString() would shift the day
const pad = (n) => String(n).padStart(2, '0');
const ymd = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const parse = (s) => { const [y, m, d] = s.split('-').map(Number); return new Date(y, m - 1, d); };
const addDays = (s, n) => { const d = parse(s); d.setDate(d.getDate() + n); return ymd(d); };
const today = () => ymd(new Date());
const isDate = (s) => /^\d{4}-\d{2}-\d{2}$/.test(s) && ymd(parse(s)) === s;

const MOOD_NAMES = ['Rough', 'Low', 'Okay', 'Good', 'Great'];

const S = {
  canPick: true,
  canBackup: false, backup: null, backupTimer: null,
  dir: '',
  entries: new Map(),
  date: today(),
  cur: { rev: null, extra: [] },
  mood: null, tags: [], starred: false,
  dirty: false, blocked: false,
  timer: null, chain: Promise.resolve(), loadToken: 0,
  calMonth: null,
  year: new Date().getFullYear(), heat: 'words',
  tab: 'write',
  filter: { q: '', tag: '', starred: false },
};

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}
function toast(text, ms = 2500) {
  const t = $('toast');
  t.textContent = text; t.classList.remove('hidden');
  clearTimeout(toast.t);
  toast.t = setTimeout(() => t.classList.add('hidden'), ms);
}
function confirmBox(text, okLabel = 'OK', danger = true) {
  return new Promise((resolve) => {
    const dlg = $('confirmDialog');
    $('confirmText').textContent = text;
    const yes = $('confirmYes'), no = $('confirmNo');
    yes.textContent = okLabel;
    yes.className = 'btn ' + (danger ? 'btn-danger' : 'btn-secondary');
    const done = (v) => { yes.onclick = no.onclick = dlg.oncancel = null; dlg.close(); resolve(v); };
    yes.onclick = () => done(true);
    no.onclick = () => done(false);
    dlg.oncancel = (e) => { e.preventDefault(); done(false); };
    dlg.showModal();
  });
}
function showNotice(text, buttons) {
  const n = $('notice');
  n.replaceChildren(el('span', null, text));
  for (const b of buttons) {
    const btn = el('button', b.alt ? 'alt' : null, b.label);
    btn.onclick = b.onClick;
    n.append(btn);
  }
  n.classList.remove('hidden');
}
const hideNotice = () => $('notice').classList.add('hidden');

function renderBanner(error) {
  $('folderPath').textContent = S.dir || '…';
  const b = $('banner');
  b.replaceChildren();
  b.classList.toggle('hidden', !error);
  if (!error) return;
  b.append(el('h3', null, 'Folder problem'), el('p', null, error));
  if (!S.canPick) return;
  const btn = el('button', null, 'Choose folder');
  btn.onclick = changeFolder;
  b.append(btn);
}

async function applyFolder(path) {
  try {
    S.dir = await invoke('set_journal_dir', { path });
    S.blocked = false; hideNotice();
    await refreshAll(true);
    toast('Journal folder changed');
  } catch (e) {
    toast(msgOf(e), 5000);
  }
}

async function changeFolder() {
  await flush();
  let picked;
  try {
    picked = await T.dialog.open({ directory: true, multiple: false, defaultPath: S.dir || undefined });
  } catch (e) {
    toast('Folder picker failed: ' + msgOf(e));
    return;
  }
  if (picked) await applyFolder(picked);
}

async function fetchEntries() {
  const list = await invoke('list_entries');
  S.entries = new Map(list.map((e) => [e.date, e]));
}

async function refreshAll(reloadDay) {
  try {
    await fetchEntries();
    renderBanner();
  } catch (e) {
    S.entries = new Map();
    renderBanner(msgOf(e));
  }
  renderStats(); renderCalendar(); renderActive();
  if (reloadDay) await loadDay(S.date, true);
}

function fillEditor(entry) {
  const ta = $('entryText');
  ta.value = entry ? entry.body : '';
  S.mood = entry ? entry.mood : null;
  S.tags = entry ? [...entry.tags] : [];
  S.starred = entry ? entry.starred : false;
  S.cur = { rev: entry ? entry.rev : null, extra: entry ? [...entry.extra] : [] };
  S.dirty = false; S.blocked = false;
  $('tagsInput').value = S.tags.join(', ');
  renderMeta();
  updateWordCount();
  setIndicator('');
}

async function loadDay(date, skipFlush) {
  if (!skipFlush) await flush();
  const token = ++S.loadToken;
  let entry = null;
  try { entry = await invoke('read_entry', { date }); }
  catch (e) { setIndicator('Error: ' + msgOf(e), 'error'); }
  if (token !== S.loadToken) return;
  S.date = date;
  S.calMonth = date.slice(0, 7);
  if (entry) S.entries.set(date, entry); else S.entries.delete(date);
  $('dateInput').value = date;
  const d = parse(date);
  $('dayName').textContent = `${d.toLocaleDateString('en-US', { weekday: 'long' })} · ${date}`;
  $('dayTitle').replaceChildren(
    document.createTextNode(d.toLocaleDateString('en-US', { day: 'numeric', month: 'long' }) + ' '),
    el('span', 'year', String(d.getFullYear())),
  );
  hideNotice();
  fillEditor(entry);
  $('deleteBtn').classList.toggle('hidden', !entry);
  renderCalendar();
}

function snapshot() {
  return {
    body: $('entryText').value,
    meta: { mood: S.mood, tags: S.tags, starred: S.starred, extra: S.cur.extra },
  };
}
function setIndicator(text, cls) {
  const i = $('saveIndicator');
  i.textContent = text;
  i.className = 'save-indicator' + (cls ? ' ' + cls : '');
}
function markDirty() {
  S.dirty = true;
  if (S.blocked) return;
  setIndicator('Saving…', 'saving');
  clearTimeout(S.timer);
  S.timer = setTimeout(() => save(), 800);
}
function save(force = false) {
  clearTimeout(S.timer);
  S.chain = S.chain.then(() => doSave(force)).catch(() => false);
  return S.chain;
}
const flush = () => (S.dirty && !S.blocked ? save() : S.chain);

async function doSave(force) {
  if (!S.dirty && !force) return true;
  if (S.blocked && !force) return false;
  const date = S.date;
  const snap = snapshot();
  S.dirty = false;
  try {
    const res = await invoke('write_entry', {
      date, body: snap.body, meta: snap.meta, expectedRev: S.cur.rev, force,
    });
    if (date !== S.date) return true;
    if (res) { S.entries.set(date, res); S.cur = { rev: res.rev, extra: res.extra }; }
    else { S.entries.delete(date); S.cur = { rev: null, extra: [] }; }
    S.blocked = false; hideNotice();
    scheduleBackup();
    $('deleteBtn').classList.toggle('hidden', !res);
    if (!S.dirty) {
      setIndicator(res ? 'Saved' : '');
      setTimeout(() => { if ($('saveIndicator').textContent === 'Saved') setIndicator(''); }, 2000);
    }
    renderStats(); renderCalendar(); renderActive();
    return true;
  } catch (e) {
    S.dirty = true;
    const m = msgOf(e);
    if (m === 'conflict') onConflict();
    else if (m === 'not-utf8') onNotUtf8();
    else setIndicator('Error: ' + m, 'error');
    return false;
  }
}

function onConflict() {
  S.blocked = true;
  setIndicator('Not saved — changed on disk', 'error');
  showNotice('This entry was changed outside Dayfile (a sync app or another editor?). Your edits are not saved yet.', [
    { label: 'Load the disk version', alt: true, onClick: () => loadDay(S.date, true) },
    { label: 'Keep mine and overwrite', onClick: () => { S.blocked = false; S.dirty = true; save(true); } },
  ]);
}
function onNotUtf8() {
  S.blocked = true;
  setIndicator('Not saved — file is not UTF-8', 'error');
  showNotice('This file uses another text encoding (probably an old .txt). Saving would convert it to UTF-8 and may garble accented characters.', [
    { label: 'Leave it alone', alt: true, onClick: () => loadDay(S.date, true) },
    { label: 'Convert and save', onClick: () => { S.blocked = false; S.dirty = true; save(true); } },
  ]);
}

async function deleteEntry() {
  if (!(await confirmBox(`Delete the entry for ${S.date}?`, 'Delete'))) return;
  clearTimeout(S.timer);
  try {
    await S.chain;
    await invoke('delete_entry', { date: S.date });
    S.entries.delete(S.date);
    fillEditor(null);
    $('deleteBtn').classList.add('hidden');
    renderStats(); renderCalendar(); renderActive();
    toast('Entry deleted');
  } catch (e) { toast('Error: ' + msgOf(e)); }
}

function updateWordCount() {
  const w = $('entryText').value.trim().split(/\s+/).filter(Boolean).length;
  $('wordCount').textContent = w ? `${w} word${w === 1 ? '' : 's'}` : '';
}
function parseTags(s) {
  const seen = new Set();
  return s.split(',').map((t) => t.trim().replace(/^#+/, '').trim())
    .filter((t) => { const k = t.toLowerCase(); return t && !seen.has(k) && seen.add(k); });
}
function renderMeta() {
  document.querySelectorAll('.mood-btn').forEach((b) => b.setAttribute('aria-pressed', String(Number(b.dataset.mood) === S.mood)));
  const s = $('starBtn');
  s.textContent = S.starred ? '★' : '☆';
  s.setAttribute('aria-pressed', String(S.starred));
}

function streaks() {
  const has = new Set(S.entries.keys());
  let cur = 0;
  // today may not be written yet
  let d = has.has(today()) ? today() : addDays(today(), -1);
  while (has.has(d)) { cur++; d = addDays(d, -1); }
  let longest = 0, run = 0, prev = null;
  for (const date of [...has].sort()) {
    run = prev && addDays(prev, 1) === date ? run + 1 : 1;
    longest = Math.max(longest, run); prev = date;
  }
  return { cur, longest };
}
function renderStats() {
  const { cur, longest } = streaks();
  let words = 0;
  for (const e of S.entries.values()) words += e.words;
  $('streakChip').textContent = cur ? `${cur}-day streak` : 'Start a streak';
  $('statEntries').textContent = S.entries.size;
  $('statStreak').textContent = cur;
  $('statLongest').textContent = longest;
  $('statWords').textContent = words.toLocaleString('en-US');
}

function renderWeek() {
  const root = $('weekStrip');
  root.replaceChildren();
  const monday = addDays(S.date, -((parse(S.date).getDay() + 6) % 7));
  const t = today();
  'MTWTFSS'.split('').forEach((letter, i) => {
    const date = addDays(monday, i);
    const b = el('button', 'week-day');
    b.setAttribute('aria-label', parse(date).toLocaleDateString('en-US', { weekday: 'long', month: 'long', day: 'numeric' }));
    b.append(el('span', 'wd-l', letter), el('span', 'wd-n', String(parse(date).getDate())), el('span', 'wd-dot'));
    if (S.entries.has(date)) b.classList.add('has');
    if (date === S.date) b.classList.add('sel');
    if (date === t) b.classList.add('today');
    b.onclick = () => loadDay(date);
    root.append(b);
  });
}

function renderCalendar() {
  renderWeek();
  const [y, m] = (S.calMonth || S.date.slice(0, 7)).split('-').map(Number);
  const root = $('calendar');
  root.replaceChildren();
  const head = el('div', 'cal-head');
  const prev = el('button', 'nav-btn', '◀'), next = el('button', 'nav-btn', '▶');
  prev.onclick = () => { const d = new Date(y, m - 2, 1); S.calMonth = `${d.getFullYear()}-${pad(d.getMonth() + 1)}`; renderCalendar(); };
  next.onclick = () => { const d = new Date(y, m, 1); S.calMonth = `${d.getFullYear()}-${pad(d.getMonth() + 1)}`; renderCalendar(); };
  head.append(prev, el('div', 'cal-title', new Date(y, m - 1, 1).toLocaleDateString('en-US', { month: 'long', year: 'numeric' })), next);
  const grid = el('div', 'cal-grid');
  ['M', 'T', 'W', 'T', 'F', 'S', 'S'].forEach((d) => grid.append(el('div', 'cal-dow', d)));
  const lead = (new Date(y, m - 1, 1).getDay() + 6) % 7;
  for (let i = 0; i < lead; i++) grid.append(el('div', 'cal-blank'));
  const days = new Date(y, m, 0).getDate();
  const t = today();
  for (let d = 1; d <= days; d++) {
    const date = `${y}-${pad(m)}-${pad(d)}`;
    const b = el('button', 'cal-day', String(d));
    if (S.entries.has(date)) b.classList.add('has');
    if (date === t) b.classList.add('today');
    if (date === S.date) b.classList.add('sel');
    b.onclick = () => { switchTab('write'); loadDay(date); };
    grid.append(b);
  }
  root.append(head, grid);
}

function highlight(text, q) {
  const frag = document.createDocumentFragment();
  const lower = text.toLowerCase(), needle = q.toLowerCase();
  if (!needle || lower.length !== text.length) { frag.append(text); return frag; }
  let i = 0, at;
  while ((at = lower.indexOf(needle, i)) !== -1) {
    frag.append(text.slice(i, at));
    frag.append(el('mark', null, text.slice(at, at + needle.length)));
    i = at + needle.length;
  }
  frag.append(text.slice(i));
  return frag;
}
function snippet(body, q) {
  const flat = body.replace(/\s+/g, ' ').trim();
  const at = q ? flat.toLowerCase().indexOf(q.toLowerCase()) : -1;
  const start = at > 80 ? at - 60 : 0;
  const s = flat.slice(start, start + 180);
  return (start ? '…' : '') + s + (start + 180 < flat.length ? '…' : '');
}
function matches(e) {
  const f = S.filter;
  if (f.starred && !e.starred) return false;
  if (f.tag && !e.tags.includes(f.tag)) return false;
  if (!f.q) return true;
  const q = f.q.toLowerCase();
  return e.body.toLowerCase().includes(q) || e.date.includes(q) || e.tags.some((t) => t.toLowerCase().includes(q));
}
function renderBrowse() {
  const tags = [...new Set([...S.entries.values()].flatMap((e) => e.tags))].sort((a, b) => a.localeCompare(b));
  const sel = $('tagFilter');
  sel.replaceChildren(new Option('All tags', ''));
  tags.forEach((t) => sel.append(new Option('#' + t, t)));
  if (tags.includes(S.filter.tag)) sel.value = S.filter.tag; else S.filter.tag = '';

  const list = [...S.entries.values()].filter(matches).sort((a, b) => b.date.localeCompare(a.date));
  $('browseCount').textContent = `${list.length} of ${S.entries.size} entr${S.entries.size === 1 ? 'y' : 'ies'}`;
  const root = $('filesList');
  root.replaceChildren();
  if (!list.length) {
    root.append(el('div', 'empty', S.entries.size ? 'No entries match.' : 'No entries yet — write your first one!'));
    return;
  }
  let month = '';
  for (const e of list) {
    const key = e.date.slice(0, 7);
    if (key !== month) {
      month = key;
      root.append(el('div', 'month-head', parse(e.date).toLocaleDateString('en-US', { month: 'long', year: 'numeric' })));
    }
    const card = el('button', 'file-item');
    const day = el('div', 'row-day');
    day.append(el('div', 'row-num', e.date.slice(8)), el('div', 'row-wd', parse(e.date).toLocaleDateString('en-US', { weekday: 'short' })));
    const body = el('div', 'row-body');
    const prev = el('div', 'file-preview');
    prev.append(highlight(snippet(e.body, S.filter.q), S.filter.q));
    const bits = [];
    if (e.mood) bits.push(MOOD_NAMES[e.mood - 1]);
    if (e.starred) bits.push('Starred');
    const meta = el('div', 'row-meta', bits.join(' · '));
    e.tags.forEach((t) => meta.append(el('span', 'tag', '#' + t)));
    body.append(prev, meta);
    card.append(day, body);
    card.onclick = () => { switchTab('write'); loadDay(e.date); };
    root.append(card);
  }
}

async function exportAll(format) {
  try {
    const path = await T.dialog.save({
      defaultPath: `dayfile-export-${format}.zip`,
      filters: [{ name: 'Zip archive', extensions: ['zip'] }],
    });
    if (!path) return;
    const n = await invoke('export_all', { path, format });
    toast(`Exported ${n} entr${n === 1 ? 'y' : 'ies'}`);
  } catch (e) { toast('Export failed: ' + msgOf(e)); }
}

function renderYear() {
  const y = S.year;
  $('yearTitle').textContent = y;
  $('heatWords').classList.toggle('active', S.heat === 'words');
  $('heatMood').classList.toggle('active', S.heat === 'mood');
  const root = $('heatmap');
  root.replaceChildren();
  const t = today();
  let count = 0;
  for (let m = 0; m < 12; m++) {
    const box = el('div', 'heat-month');
    box.append(el('div', 'heat-label', new Date(y, m, 1).toLocaleDateString('en-US', { month: 'short' })));
    const grid = el('div', 'heat-grid');
    for (let i = (new Date(y, m, 1).getDay() + 6) % 7; i > 0; i--) grid.append(el('span'));
    for (let d = 1, n = new Date(y, m + 1, 0).getDate(); d <= n; d++) {
      const date = `${y}-${pad(m + 1)}-${pad(d)}`;
      const cell = el('button', 'heat-cell');
      const e = S.entries.get(date);
      cell.title = date;
      if (e) {
        count++;
        if (S.heat === 'mood') cell.classList.add('m' + (e.mood || 0));
        else cell.classList.add('l' + (e.words >= 300 ? 4 : e.words >= 150 ? 3 : e.words >= 50 ? 2 : 1));
        if (e.starred) cell.classList.add('star');
        cell.title = `${date} · ${e.words} words` + (e.mood ? ` · ${MOOD_NAMES[e.mood - 1]}` : '');
      }
      if (date > t) cell.classList.add('future');
      if (date === t) cell.classList.add('today');
      cell.onclick = () => { switchTab('write'); loadDay(date); };
      grid.append(cell);
    }
    box.append(grid);
    root.append(box);
  }
  const lg = $('heatLegend');
  lg.replaceChildren();
  const cls = S.heat === 'mood' ? ['m1', 'm2', 'm3', 'm4', 'm5'] : ['', 'l1', 'l2', 'l3', 'l4'];
  lg.append(document.createTextNode(S.heat === 'mood' ? 'Rough' : 'Less'));
  cls.forEach((c) => lg.append(el('span', 'heat-cell ' + c)));
  lg.append(document.createTextNode(S.heat === 'mood' ? 'Great' : 'More'));
  const days = (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0 ? 366 : 365;
  $('heatSummary').textContent = `${count} entr${count === 1 ? 'y' : 'ies'} in ${y} · ${Math.round((count / days) * 100)}% of days`;
}

function renderActive() {
  if (S.tab === 'browse') renderBrowse();
  else if (S.tab === 'year') renderYear();
}
async function switchTab(tab) {
  if (tab !== 'write') await flush();
  S.tab = tab;
  document.querySelectorAll('.tab-btn').forEach((b) => b.classList.toggle('active', b.dataset.tab === tab));
  for (const t of ['write', 'browse', 'year', 'settings']) $('tab-' + t).classList.toggle('hidden', t !== tab);
  renderActive();
}

// sync clients and other editors may have touched files while we were in the background
async function onFocus() {
  if (S.dir === '') return;
  try {
    await fetchEntries();
    renderStats(); renderCalendar(); renderActive();
    if (!S.dirty && !S.blocked && S.tab === 'write') {
      const disk = S.entries.get(S.date);
      if ((disk ? disk.rev : null) !== S.cur.rev) { await loadDay(S.date, true); toast('Updated from disk'); }
    }
  } catch (_) {}
}

function wire() {
  MOOD_NAMES.forEach((name, i) => {
    const b = el('button', 'mood-btn', name);
    b.dataset.mood = i + 1;
    b.setAttribute('aria-pressed', 'false');
    b.onclick = () => { S.mood = S.mood === i + 1 ? null : i + 1; renderMeta(); markDirty(); };
    $('moods').append(b);
  });

  $('entryText').addEventListener('input', () => { updateWordCount(); markDirty(); });
  $('entryText').addEventListener('blur', () => { if (S.dirty && !S.blocked) save(); });
  $('tagsInput').addEventListener('input', (e) => { S.tags = parseTags(e.target.value); markDirty(); });
  $('tagsInput').addEventListener('blur', (e) => { e.target.value = S.tags.join(', '); });
  $('starBtn').onclick = () => { S.starred = !S.starred; renderMeta(); markDirty(); };
  $('deleteBtn').onclick = deleteEntry;

  $('prevDay').onclick = () => loadDay(addDays(S.date, -1));
  $('nextDay').onclick = () => loadDay(addDays(S.date, 1));
  $('todayBtn').onclick = () => loadDay(today());
  $('dateInput').addEventListener('change', (e) => { if (isDate(e.target.value)) loadDay(e.target.value); });

  document.querySelectorAll('.tab-btn').forEach((b) => { b.onclick = () => switchTab(b.dataset.tab); });

  $('searchInput').addEventListener('input', (e) => { S.filter.q = e.target.value.trim(); renderBrowse(); });
  $('tagFilter').addEventListener('change', (e) => { S.filter.tag = e.target.value; renderBrowse(); });
  $('starFilter').onclick = (e) => {
    S.filter.starred = !S.filter.starred;
    e.currentTarget.setAttribute('aria-pressed', String(S.filter.starred));
    renderBrowse();
  };
  $('changeFolder').onclick = changeFolder;
  document.querySelectorAll('[data-theme-opt]').forEach((b) => {
    b.onclick = () => {
      try { localStorage.setItem('dayfile-theme', b.dataset.themeOpt); } catch (_) {}
      applyTheme(b.dataset.themeOpt);
    };
  });
  $('exportTxt').onclick = () => exportAll('txt');
  $('exportMd').onclick = () => exportAll('md');
  $('exportJson').onclick = () => exportAll('json');
  $('restoreBtn').onclick = restoreBackup;
  document.addEventListener('visibilitychange', () => { if (document.hidden && S.backupTimer) runBackup(); });

  $('prevYear').onclick = () => { S.year--; renderYear(); };
  $('nextYear').onclick = () => { S.year++; renderYear(); };
  $('heatWords').onclick = () => { S.heat = 'words'; renderYear(); };
  $('heatMood').onclick = () => { S.heat = 'mood'; renderYear(); };

  document.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); if (S.dirty) save(); }
    if (e.altKey && S.tab === 'write' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      e.preventDefault(); loadDay(addDays(S.date, e.key === 'ArrowLeft' ? -1 : 1));
    }
  });
  document.addEventListener('visibilitychange', () => { if (document.hidden && S.dirty && !S.blocked) save(); });
  window.addEventListener('blur', () => { if (S.dirty && !S.blocked) save(); });
  window.addEventListener('focus', onFocus);
  window.addEventListener('pagehide', () => { if (S.dirty && !S.blocked) save(); });
}

function applyTheme(pref) {
  if (pref === 'light' || pref === 'dark') document.documentElement.dataset.theme = pref;
  else delete document.documentElement.dataset.theme;
  document.querySelectorAll('[data-theme-opt]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.themeOpt === pref)));
}
function savedTheme() {
  try { return localStorage.getItem('dayfile-theme') || 'system'; } catch (_) { return 'system'; }
}

function scheduleBackup(delay = 20000) {
  if (!S.canBackup) return;
  clearTimeout(S.backupTimer);
  S.backupTimer = setTimeout(runBackup, delay);
}

async function runBackup() {
  clearTimeout(S.backupTimer);
  S.backupTimer = null;
  // let a pending save land first so the zip has the latest text
  await flush();
  try {
    S.backup = await invoke('backup_now', { day: today() });
  } catch (e) {
    toast('Backup failed: ' + msgOf(e), 5000);
    try { S.backup = await invoke('backup_status'); } catch (_) {}
  }
  renderBackup();
}

function renderBackup() {
  const p = $('backupStatus'), b = S.backup;
  if (!S.canBackup || !b) return;
  p.textContent = b.error ? 'Last backup failed: ' + b.error
    : b.last ? 'Last backed up ' + new Date(b.last).toLocaleString() : 'No backup yet';
  p.classList.remove('hidden');
}

async function restoreBackup() {
  try {
    const picked = await T.dialog.open({ multiple: false, filters: [{ name: 'Zip archive', extensions: ['zip'] }] });
    if (!picked) return;
    const r = await invoke('restore_backup', await T.fs.readFile(picked));
    hideNotice();
    await refreshAll(true);
    const n = r.restored;
    toast(`Restored ${n} entr${n === 1 ? 'y' : 'ies'}` + (r.skipped ? `, ${r.skipped} already existed` : ''), 4000);
    scheduleBackup(2000);
  } catch (e) { toast('Restore failed: ' + msgOf(e), 4000); }
}

async function init() {
  wire();
  applyTheme(savedTheme());
  try { S.canPick = await invoke('can_pick_folder'); } catch (_) {}
  if (!S.canPick) {
    $('changeFolder').classList.add('hidden');
    $('folderNote').classList.remove('hidden');
  }
  try { S.dir = await invoke('journal_dir'); } catch (e) { renderBanner(msgOf(e)); }
  await refreshAll(true);
  try { S.canBackup = await invoke('can_backup'); } catch (_) {}
  if (S.canBackup) {
    $('backupNote').classList.remove('hidden');
    try { S.backup = await invoke('backup_status'); renderBackup(); } catch (_) {}
    if (S.entries.size === 0) {
      showNotice('Reinstalled? Restore your entries from a backup zip.', [
        { label: 'Restore…', onClick: restoreBackup },
        { label: 'Dismiss', alt: true, onClick: hideNotice },
      ]);
    } else scheduleBackup(2000);
  }
}

init();
})();
