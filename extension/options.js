// The options page: port, token and the first language of the right-click menu.

const $ = (id) => document.getElementById(id);

function status(text, kind) {
  const el = $('status');
  el.textContent = text;
  el.className = kind || '';
}

async function loadLanguages(port, token, selected) {
  const res = await fetch(`http://127.0.0.1:${port}/v1/languages`, { headers: { Authorization: `Bearer ${token}` } });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || `Corydoras answered ${res.status}.`);
  const select = $('language');
  select.length = 1;
  for (const l of data.languages || []) {
    const o = document.createElement('option');
    o.value = l.id;
    o.textContent = `${l.name} (${l.items} images)`;
    select.appendChild(o);
  }
  select.value = selected && [...select.options].some((o) => o.value === selected) ? selected : '';
  return data;
}

async function init() {
  const s = await chrome.storage.local.get(['port', 'token', 'language']);
  $('port').value = s.port || 47821;
  $('token').value = s.token || '';
  if (s.token) {
    try {
      const data = await loadLanguages($('port').value, s.token, s.language);
      status(`Connected to Corydoras ${data.version}.`, 'good');
    } catch (e) {
      status(String(e.message || e), 'bad');
    }
  }
}

$('save').addEventListener('click', async () => {
  const port = Number($('port').value) || 47821;
  const token = $('token').value.trim();
  if (!token) return status('Paste the token from Corydoras › Settings › Browser extension.', 'bad');
  try {
    const data = await loadLanguages(port, token, $('language').value);
    await chrome.storage.local.set({ port, token, language: $('language').value });
    await chrome.runtime.sendMessage({ type: 'refresh' });
    status(`Saved. Connected to Corydoras ${data.version} with ${data.languages.length} languages. Right-click an image or a page to add it.`, 'good');
  } catch (e) {
    const offline = e instanceof TypeError;
    status(offline ? 'Corydoras does not answer on this port. Open the app, turn on the bridge in its Settings, and check the port.' : String(e.message || e), 'bad');
  }
});

$('language').addEventListener('change', async () => {
  await chrome.storage.local.set({ language: $('language').value });
  await chrome.runtime.sendMessage({ type: 'refresh' });
});

$('show').addEventListener('click', () => {
  const t = $('token');
  t.type = t.type === 'password' ? 'text' : 'password';
  $('show').textContent = t.type === 'password' ? 'Show the token' : 'Hide the token';
});

init();
