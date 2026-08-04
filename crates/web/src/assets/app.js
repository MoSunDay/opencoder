// app.js — session list, send/interrupt, model/agent switching, fork/compact,
// image upload, keyboard handling, init. State + render helpers live in render.js.
function currentId() { return cur; }

// ── session list ────────────────────────────────────────────────────────────
async function refresh() {
  const search = $('#search').value.trim();
  const qs = search ? '&search=' + encodeURIComponent(search) : '';
  const r = await fetch(withToken('/api/sessions?limit=50' + qs));
  const j = await r.json();
  const list = j.sessions || [];
  const html = list.map(s => {
    const active = s.id === cur ? ' active' : '';
    const title = s.title || s.id.slice(0, 8);
    const skill = s.skill ? ' <span style="color:var(--pl)">$' + esc(s.skill) + '</span>' : '';
    return '<div class="sess' + active + '" data-id="' + s.id + '" onclick="selectSession(\'' + s.id + '\')">' +
      '<span>' + esc(title) + skill + '</span>' +
      '<button class="del" onclick="event.stopPropagation();delSession(\'' + s.id + '\')" title="delete">&#x2715;</button>' +
      '</div>';
  }).join('');
  $('#sess-list').innerHTML = html || '<div style="color:var(--mut);padding:8px;font-size:12px">no sessions</div>';
}

async function selectSession(id) {
  cur = id;
  if (es) { es.close(); es = null; }
  busy = false;
  $('#log').innerHTML = '';
  updateSendBtn();
  await load();
  await refresh();
  openStream();
}

async function newSession() {
  const r = await fetch(withToken('/api/sessions'), {
    method: 'POST', headers: authHeaders({ 'content-type': 'application/json' }), body: JSON.stringify({})
  });
  const j = await r.json();
  if (j.id) await selectSession(j.id);
}

async function delSession(id) {
  if (!confirm('Delete this session?')) return;
  await fetch(withToken('/api/sessions/' + id), { method: 'DELETE', headers: authHeaders({}) });
  if (cur === id) { cur = null; $('#log').innerHTML = ''; }
  refresh();
}

// ── send prompt ─────────────────────────────────────────────────────────────
async function send(delivery) {
  if (!cur) { await newSession(); if (!cur) return; }
  const t = $('#msg').value.trim();
  if (!t) return;
  busy = true; updateSendBtn();
  $('#msg').value = ''; $('#msg').style.height = 'auto';

  const u = document.createElement('div'); u.className = 'm user';
  u.innerHTML = '<div class="r">user</div><div class="b"></div>';
  u.querySelector('.b').textContent = t;
  pendingImages.forEach(src => { const img = document.createElement('img'); img.className = 'img-att'; img.src = src; img.style.maxHeight = '80px'; u.appendChild(img); });
  $('#log').appendChild(u); scrollEnd();

  const body = { prompt: t, delivery: delivery || 'steer' };
  if (pendingImages.length) { body.images = pendingImages.slice(); pendingImages = []; renderImgPreview(); }

  try {
    const r = await fetch(withToken('/api/sessions/' + cur + '/prompt'), {
      method: 'POST', headers: authHeaders({ 'content-type': 'application/json' }), body: JSON.stringify(body)
    });
    const j = await r.json();
    if (!j.ok && j.error) { alert(j.error); busy = false; updateSendBtn(); return; }
    if (!es) openStream();
  } catch (e) { alert(e); busy = false; updateSendBtn(); }
}

function interrupt() {
  if (!cur) return;
  fetch(withToken('/api/sessions/' + cur + '/interrupt'), { method: 'POST', headers: authHeaders({}) });
}

function updateSendBtn() {
  const btn = $('#send');
  if (busy) { btn.textContent = 'Interrupt'; btn.classList.add('interrupt'); btn.onclick = interrupt; }
  else { btn.textContent = 'Send'; btn.classList.remove('interrupt'); btn.onclick = () => send('steer'); }
}

function updateModeDisplay() { const el = $('#mode'); if (el) el.value = mode; }

// ── model / agent switching ─────────────────────────────────────────────────
async function switchModel() {
  const m = $('#model').value.trim();
  if (!m || !cur) return;
  await fetch(withToken('/api/sessions/' + cur + '/model'), {
    method: 'POST', headers: authHeaders({ 'content-type': 'application/json' }), body: JSON.stringify({ value: m })
  });
}

async function switchAgent() {
  mode = $('#mode').value;
  if (!cur) return;
  await fetch(withToken('/api/sessions/' + cur + '/agent'), {
    method: 'POST', headers: authHeaders({ 'content-type': 'application/json' }), body: JSON.stringify({ value: mode })
  });
}

// ── fork / compact ──────────────────────────────────────────────────────────
async function forkSession() {
  if (!cur) return;
  const r = await fetch(withToken('/api/sessions/' + cur + '/fork'), { method: 'POST', headers: authHeaders({}) });
  const j = await r.json();
  if (j.id) await selectSession(j.id);
}

async function compactSession() {
  if (!cur) return;
  await fetch(withToken('/api/sessions/' + cur + '/compact'), { method: 'POST', headers: authHeaders({}) });
  if (!es) openStream();
}

// ── image paste / upload ────────────────────────────────────────────────────
function renderImgPreview() {
  const c = $('#img-preview');
  c.innerHTML = '';
  pendingImages.forEach((src, i) => {
    const wrap = document.createElement('div'); wrap.className = 'pi';
    const img = document.createElement('img'); img.src = src;
    const rm = document.createElement('button'); rm.className = 'rm'; rm.textContent = '\u2715';
    rm.onclick = () => { pendingImages.splice(i, 1); renderImgPreview(); };
    wrap.appendChild(img); wrap.appendChild(rm); c.appendChild(wrap);
  });
}

function handlePaste(e) {
  const items = e.clipboardData ? e.clipboardData.items : [];
  for (const it of items) {
    if (!it.type.startsWith('image/')) continue;
    const f = it.getAsFile();
    if (!f) continue;
    const reader = new FileReader();
    reader.onload = () => { pendingImages.push(reader.result); renderImgPreview(); };
    reader.readAsDataURL(f);
  }
}

function handleFileSelect(e) {
  const files = e.target.files || [];
  for (const f of files) {
    if (!f.type.startsWith('image/')) continue;
    const reader = new FileReader();
    reader.onload = () => { pendingImages.push(reader.result); renderImgPreview(); };
    reader.readAsDataURL(f);
  }
}

// ── init ────────────────────────────────────────────────────────────────────
document.addEventListener('paste', handlePaste);
$('#search').addEventListener('input', () => { clearTimeout(window._st); window._st = setTimeout(refresh, 200); });
$('#msg').addEventListener('keydown', e => {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); if (!busy) send('steer'); }
  else if (e.key === 'Enter' && e.shiftKey) { e.preventDefault(); if (!busy) send('queue'); }
  else if (e.key === 'Escape' && busy) { interrupt(); }
});
$('#msg').addEventListener('input', function () { this.style.height = 'auto'; this.style.height = Math.min(this.scrollHeight, 120) + 'px'; });
refresh();
