// questions.js - runtime question cards. Questions are TRANSIENT state (the
// live hub): they render while a run is asking, never on transcript replay
// (replayed tool_end blocks already cover the answered record).
var qTimer = null;
var qInflight = false;

// Immediate poll + (re)start the 1.5s interval. Called on prompt submit
// (setBusy(true)) and on tool_start name=question (chat.js).
function questionsKick() {
  pollQuestions();
  if (!qTimer && cur) { qTimer = setInterval(pollQuestions, 1500); }
}
function startQuestionPoll() { questionsKick(); }
function stopQuestionPoll(clear) {
  clearInterval(qTimer);
  qTimer = null;
  if (clear) { clearQuestionCards(); }
}
function clearQuestionCards() { $('#questions').innerHTML = ''; }

async function pollQuestions() {
  if (!cur || document.hidden || qInflight) { return; }
  qInflight = true;
  var j = null;
  try {
    j = await apiGet('/api/sessions/' + cur + '/questions');
  } catch (e) {
    qInflight = false;
    return; // transient poll failure: next tick retries
  }
  qInflight = false;
  renderQuestionCards((j && j.questions) || []);
}

// Reconcile rendered cards with the polled list WITHOUT rebuilding existing
// cards, so a free-text answer being typed survives the 1.5s refresh.
function renderQuestionCards(qs) {
  var c = $('#questions');
  var seen = {};
  for (var i = 0; i < qs.length; i++) {
    var q = qs[i];
    seen[q.id] = true;
    if (!c.querySelector('[data-qid="' + q.id + '"]')) { c.appendChild(mkQuestionCard(q)); }
  }
  var kids = c.children;
  for (var k = kids.length - 1; k >= 0; k--) {
    if (!seen[kids[k].getAttribute('data-qid')]) { kids[k].remove(); }
  }
  if (!kids.length) { stopQuestionPoll(false); } // nothing pending: stop polling
}

function mkQuestionCard(q) {
  var card = document.createElement('div');
  card.className = 'q-card';
  card.setAttribute('data-qid', q.id);

  var txt = document.createElement('div');
  txt.className = 'q-text';
  txt.textContent = '\u2753 ' + (q.question || '');
  card.appendChild(txt);

  if (q.options && q.options.length) {
    var opts = document.createElement('div');
    opts.className = 'q-opts';
    q.options.forEach(function (opt) {
      var b = document.createElement('button');
      b.textContent = opt;
      b.onclick = function () { answerQuestion(q.id, String(opt), card); };
      opts.appendChild(b);
    });
    card.appendChild(opts);
  }

  var free = document.createElement('div');
  free.className = 'q-free';
  var inp = document.createElement('input');
  inp.type = 'text';
  inp.placeholder = 'answer...';
  var ans = document.createElement('button');
  ans.textContent = 'Answer';
  ans.onclick = function () { answerQuestion(q.id, inp.value, card); };
  inp.addEventListener('keydown', function (e) {
    if (e.key === 'Enter') { e.preventDefault(); answerQuestion(q.id, inp.value, card); }
  });
  free.appendChild(inp);
  free.appendChild(ans);
  card.appendChild(free);

  var skip = document.createElement('button');
  skip.className = 'q-skip';
  skip.textContent = 'skip';
  skip.onclick = function () { skipQuestion(q.id, card); };
  card.appendChild(skip);
  return card;
}

// Optimistic resolve: spinner state now, removal on the next poll (or here).
async function answerQuestion(id, answer, card) {
  answer = (answer || '').trim();
  if (!answer) { return; }
  card.classList.add('answered');
  card.querySelectorAll('button,input').forEach(function (el) { el.disabled = true; });
  try {
    await apiSend('POST', '/api/sessions/' + cur + '/questions/' + id + '/answer', { answer: answer });
    card.remove();
    pollQuestions();
  } catch (e) {
    card.classList.remove('answered');
    card.querySelectorAll('button,input').forEach(function (el) { el.disabled = false; });
    alert(e.error || e);
  }
}

async function skipQuestion(id, card) {
  try {
    await apiSend('POST', '/api/sessions/' + cur + '/questions/' + id + '/skip');
    card.remove();
    pollQuestions();
  } catch (e) { alert(e.error || e); }
}

// Re-poll promptly when the tab becomes visible again.
visibleAgain.push(function () { if (qTimer) { pollQuestions(); } });
