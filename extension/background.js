// Corydoras extension: right-click an image or a page to add it to a design
// language. Every request goes to the Corydoras app on this computer
// (127.0.0.1) and carries the token from Corydoras › Settings › Browser extension.

const DEFAULTS = { port: 47821, token: '', language: '' };

async function settings() {
  return { ...DEFAULTS, ...(await chrome.storage.local.get(['port', 'token', 'language'])) };
}

// Calls the bridge of the app. Without a body it is a GET.
async function call(path, body) {
  const { port, token } = await settings();
  if (!token) {
    throw new Error('Open the options of this extension and paste the token from Corydoras › Settings › Browser extension.');
  }
  let res;
  try {
    res = await fetch(`http://127.0.0.1:${port}${path}`, {
      method: body ? 'POST' : 'GET',
      headers: { Authorization: `Bearer ${token}`, ...(body ? { 'Content-Type': 'application/json' } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
  } catch {
    throw new Error('Corydoras does not answer. Open the app, and turn on “Let the browser extension add references” in its Settings.');
  }
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || `Corydoras answered ${res.status}.`);
  return data;
}

function notify(message, failed) {
  chrome.notifications.create({
    type: 'basic',
    iconUrl: 'icons/128.png',
    title: failed ? 'Corydoras could not add it' : 'Corydoras',
    message,
  });
}

// Menu IDs: "image|<language id>" and "page|<language id>". An empty ID means
// the language that the app uses for screen captures.
async function buildMenus() {
  await chrome.contextMenus.removeAll();
  let languages = [];
  try {
    languages = (await call('/v1/languages')).languages || [];
  } catch {
    // The app is closed or the token is missing: the two main commands still show,
    // and a click explains what to do.
  }
  const { language } = await settings();
  const preferred = languages.find((l) => l.id === language);
  const groups = [
    { key: 'image', title: 'Add image to Corydoras', contexts: ['image'] },
    { key: 'page', title: 'Add page to Corydoras sources', contexts: ['page', 'link'] },
  ];
  for (const g of groups) {
    chrome.contextMenus.create({ id: g.key, title: g.title, contexts: g.contexts });
    chrome.contextMenus.create({
      id: `${g.key}|${preferred ? preferred.id : ''}`,
      parentId: g.key,
      title: preferred ? preferred.name : 'The capture language of the app',
      contexts: g.contexts,
    });
    const others = languages.filter((l) => !preferred || l.id !== preferred.id);
    if (others.length > 0) {
      chrome.contextMenus.create({ id: `${g.key}-sep`, parentId: g.key, type: 'separator', contexts: g.contexts });
    }
    for (const l of others) {
      chrome.contextMenus.create({ id: `${g.key}|${l.id}`, parentId: g.key, title: l.name, contexts: g.contexts });
    }
  }
}

chrome.runtime.onInstalled.addListener(buildMenus);
chrome.runtime.onStartup.addListener(buildMenus);
chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());

// The options page asks for a new menu after a change.
chrome.runtime.onMessage.addListener((msg, _sender, respond) => {
  if (msg && msg.type === 'refresh') {
    buildMenus().then(() => respond({ ok: true }));
    return true;
  }
  return false;
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const [kind, language] = String(info.menuItemId).split('|');
  if (language === undefined) return;
  try {
    if (kind === 'image') {
      const src = info.srcUrl || '';
      if (src.startsWith('blob:')) {
        throw new Error('This image exists only inside the page. Drag it onto the Board, or capture it with the Corydoras shortcut.');
      }
      const r = await call('/v1/items', { language, image_url: src, page_url: info.pageUrl || '', title: '' });
      notify(r.duplicate ? `“${r.language}” has this image already.` : `Image added to “${r.language}”. The analysis starts.`);
    } else if (kind === 'page') {
      const url = info.linkUrl || info.pageUrl || (tab && tab.url) || '';
      const r = await call('/v1/sources', { language, url });
      notify(`“${r.title}” added to the sources of “${r.language}”.`);
    }
  } catch (e) {
    notify(String(e.message || e), true);
  }
  // New languages show up in the menu the next time.
  buildMenus();
});
