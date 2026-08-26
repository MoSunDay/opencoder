#!/usr/bin/env node
// Headless runtime acceptance for the embedded web frontend: loads the REAL
// asset scripts (api/sse/sessions/chat/composer/questions/queue_panel/
// subagent_view/bg_panel/settings) into a vm through the shared dom_shim,
// then asserts the runtime behaviors the static html.rs tests cannot see:
// question closed loop, queue panel list/reorder/delete, model dropdown,
// SSE reconnect badge, composer send path, subagent cards + bg panel.
// The DOM shim / mock fetch / EventSource stub live in ./dom_shim.mjs.

import { createShim, reporter, sleep } from './dom_shim.mjs';

const {
  byId, qsa, state, calls, sandbox, ES, dispatchSse: dispatch,
} = await createShim({
  names: ['api', 'sse', 'sessions', 'chat', 'composer', 'questions',
    'queue_panel', 'subagent_view', 'bg_panel', 'settings'],
});
sandbox.cur = 's1'; // select the mock session (sidebar click equivalent)

const R = reporter('FRONTEND SMOKE');
const ok = (c, m) => R.ok(c, m);
state.questions = [{ id: 'q1', question: 'which db?', options: ['pg', 'mysql'] }];
await sandbox.pollQuestions();
const Q = byId('questions');
let card = qsa(Q, '[data-qid="q1"]')[0];
ok(card, 'question card rendered with data-qid');
ok(qsa(card, '.q-text')[0].textContent.includes('which db?'), 'card shows the question text');
const optPg = qsa(card, 'button').find((b) => b.textContent === 'pg');
ok(!!optPg, 'option buttons rendered');
optPg.onclick();
await sleep(50);
ok(calls('POST', /questions\/q1\/answer$/).length === 1, 'answer POSTed to /questions/q1/answer');
ok(calls('POST', /questions\/q1\/answer$/)[0].body.answer === 'pg', 'answer body carries the option');
ok(qsa(Q, '[data-qid="q1"]').length === 0, 'card removed after answering');
state.questions = [{ id: 'q2', question: 'free text?' }];
await sandbox.pollQuestions();
card = qsa(Q, '[data-qid="q2"]')[0];
qsa(card, '.q-skip')[0].onclick();
await sleep(50);
ok(calls('POST', /questions\/q2\/skip$/).length === 1, 'skip POSTs /questions/q2/skip');
ok(qsa(Q, '[data-qid]').length === 0, 'skip clears the card');

// S2: queue panel — steers first, badge count, reorder, delete.
console.log('S2 queue panel');
state.inputs.steer = [{ seq: 7, prompt: 'steer the run', delivery: 'steer' }];
state.inputs.queue = [
  { seq: 1, prompt: 'first queued task', delivery: 'queue' },
  { seq: 2, prompt: 'second queued task', delivery: 'queue' },
];
await sandbox.refreshQueuePanel(true);
const rows = () => qsa(byId('qp-list'), '.qp-item');
ok(rows().length === 3, 'queue panel lists steer + queue rows');
ok(rows()[0].textContent.includes('steer the run'), 'steer row leads the list');
ok(qsa(rows()[1], '.qp-badge')[0].textContent === 'queue', 'delivery badge rendered');
ok(byId('qcount').textContent === '2', 'qcount badge counts queue inputs only');
ok(qsa(rows()[0], '.qp-move')[0].disabled === true, 'first row up disabled');
qsa(rows()[2], '.qp-move')[0].onclick(); // up on seq 2 → swap with seq 1
await sleep(50);
ok(calls('POST', /inputs\/reorder$/).length === 1, 'reorder POSTed');
ok(rows()[1].textContent.includes('second queued task'), 'rows re-ordered after swap');
qsa(rows()[0], '.qp-del')[0].onclick(); // delete the steer row (seq 7)
await sleep(50);
ok(calls('DELETE', /inputs\/7$/).length === 1, 'delete DELETEs /inputs/7');
ok(rows().length === 2, 'row disappears after delete');

// S3: model dropdown — catalog + custom fallback.
console.log('S3 model dropdown');
await sandbox.loadModels();
const sel = byId('model-select');
const vals = sel.children.map((o) => o.value);
ok(JSON.stringify(vals) === JSON.stringify(['a/b', 'x/y', '__custom__']),
  `dropdown options = catalog + custom (${vals.join(',')})`);
sandbox.setModelDisplay('zzz/unknown');
ok(sel.value === '__custom__' && byId('model').style.display === ''
  && byId('model').value === 'zzz/unknown', 'unknown model falls back to free-text input');
sandbox.setModelDisplay('x/y');
ok(sel.value === 'x/y' && byId('model').style.display === 'none', 'known model hides the input');

// S4: composer send path — optimistic echo + prompt POST + busy toggle.
console.log('S4 composer send');
byId('msg').value = 'hello smoke';
await sandbox.send('queue');
ok(calls('POST', /\/prompt$/)[0].body.prompt === 'hello smoke'
  && calls('POST', /\/prompt$/)[0].body.delivery === 'queue', 'prompt POSTed with delivery queue');
ok(qsa(byId('log'), '.m')[0].textContent.includes('hello smoke'), 'optimistic user echo rendered');
ok(byId('send').textContent === 'Interrupt' && sandbox.busy === true, 'busy state flips send button');
dispatch(ES[ES.length - 1], 'done', {});
await sleep(50);
ok(sandbox.busy === false && byId('send').textContent === 'Send', 'done event resets busy');

// S5: mode controls are committed only after server confirmation and stay
// disabled throughout a running drain.
console.log('S5 running mode gate');
sandbox.mode = 'act';
sandbox.updateModeDisplay();
sandbox.setBusy(true);
ok(byId('mode').disabled && byId('handoff').disabled, 'mode and handoff disabled while busy');
byId('msg').value = '/plan later';
const beforeBusyPrompt = calls('POST', /\/prompt$/).length;
await sandbox.send('queue');
ok(calls('POST', /\/prompt$/).length === beforeBusyPrompt, 'busy text mode command sends no request');
ok(byId('msg').value === '/plan later', 'busy text mode command preserves composer input');
byId('msg').value = '';
const beforeBusySwitch = calls('POST', /\/agent$/).length;
const beforeBusyHandoff = calls('POST', /\/handoff$/).length;
byId('mode').value = 'plan';
await sandbox.switchAgent();
await sandbox.handoffSession();
ok(calls('POST', /\/agent$/).length === beforeBusySwitch, 'busy mode switch sends no request');
ok(calls('POST', /\/handoff$/).length === beforeBusyHandoff, 'busy handoff sends no request');
ok(sandbox.mode === 'act' && byId('mode').value === 'act', 'busy select rolls back to committed mode');
sandbox.setBusy(false);
state.agentStatus = 409;
byId('mode').value = 'plan';
await sandbox.switchAgent();
ok(sandbox.mode === 'act' && byId('mode').value === 'act', 'server rejection preserves committed mode');
state.agentStatus = 200;
byId('mode').value = 'plan';
await sandbox.switchAgent();
ok(sandbox.mode === 'plan' && byId('mode').value === 'plan', 'successful switch commits server mode');
ok(!byId('mode').disabled && !byId('handoff').disabled, 'idle controls re-enabled after request');

// S6: SSE reconnect — badge on error, resume from /seq, reset on event,
// persistent banner after max attempts.
console.log('S6 sse reconnect');
const badge = byId('reconnect');
const es1 = ES[ES.length - 1];
es1.onerror();
ok(badge.style.display === '', 'reconnect badge visible on stream error');
await sleep(1300); // backoff 1s → /seq → reopen with ?after=
const es2 = ES[ES.length - 1];
ok(es2 !== es1 && es2.url.includes('after=5'), `reopened from persisted seq (${es2.url})`);
dispatch(es2, 'text_delta', { text: 'x' });
ok(badge.style.display === 'none', 'badge hidden once events flow again');
sandbox.sseAttempts = 5; // white-box: jump to the last allowed attempt
es2.onerror();
ok(byId('reconnect-fail').style.display === '', 'persistent fail banner after max attempts');


// S7: subagent drill-down + background-process panel (subagent_view.js,
// bg_panel.js). Cards restored from the durable task list after a transcript
// reload; the delegated expand click opens the child transcript drawer; the
// settings panel lists and stops background processes.
console.log('S7 subagent view + bg panel');
const fire = (el, kind, ev) => (el._listeners && el._listeners[kind] || []).forEach((fn) => fn(ev || { target: el }));
state.subagents = [
  { id: 't1', kind: 'explore', status: 'completed', child_session_id: 's1', prompt: 'map the crates', result: '9 crates' },
  { id: 't2', kind: 'build', status: 'running', child_session_id: 's2', prompt: 'add the endpoint' },
];
state.messagesBySession['s1'] = [
  { role: 'user', blocks: [{ type: 'text', text: 'map the crates' }] },
  { role: 'assistant', blocks: [{ type: 'text', text: 'child reply: found 9' }] },
];
await sandbox.loadTranscript(); // chat.js snapshot paint + subagent restore
const log = byId('log');
const cards = qsa(log, '.subagent-card');
ok(cards.length === 2, `historical subagent cards restored after reload (${cards.length})`);
// (the DOM shim has no compound-class selector support: match via class set)
const doneCard = cards.find((c) => qsa(c, '.sa-tail').some((t) => t._cls.has('ok'))) || null;
ok(!!doneCard && doneCard.textContent.includes('9 crates'), 'completed card carries its result tail');
const runningCard = cards.find((c) => !qsa(c, '.sa-tail').length) || null;
ok(!!runningCard && qsa(runningCard, '.sa-dot').some((d) => d._cls.has('running')),
  'running card keeps the pulsing dot');
if (!doneCard || !runningCard) {
  R.fail('S7 preconditions unmet — skipping expand flow');
} else {
ok(!qsa(doneCard, '.sa-steer').length, 'completed card has no steer button');
const expand = qsa(doneCard, '.sa-expand')[0];
ok(!!expand && expand.dataset.child === 's1', 'expand button carries the child session id');
fire(log, 'click', { target: expand }); // delegated through #log
await sleep(50);
const drawer = byId('sa-drawer');
ok(!!drawer, 'child transcript drawer opened');
ok(drawer.textContent.includes('child reply: found 9'), 'drawer renders the child session transcript');
ok(drawer.textContent.includes('child: s1'), 'drawer surfaces the child session id');
ok(calls('GET', /\/sessions\/s1\/messages$/).length >= 1, 'child transcript fetched via the messages endpoint');
qsa(drawer, '.sa-drawer-hdr button')[0].onclick();
ok(!byId('sa-drawer') && !byId('sa-backdrop'), 'close button tears the drawer down');
}

state.bgProcs = [{ pid: 4242, output_path: '/tmp/bg-4242.log' }];
await sandbox.refreshBgPanel();
const bgBox = byId('bg-list');
ok(bgBox.textContent.includes('pid 4242'), 'bg panel lists the live process');
qsa(bgBox, 'button').find((b) => b.textContent === 'stop all').onclick();
await sleep(50);
ok(calls('POST', /\/api\/bg\/stop$/).length === 1, 'stop-all POSTs /api/bg/stop');
ok(qsa(byId('log'), '.sys-chip').some((c) => c.textContent.includes('stopped 1')),
  'stop result surfaced as a system chip');
ok(!qsa(byId('bg-list'), '.bg-row').length, 'panel empties after stop-all');


R.finish();
