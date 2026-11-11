import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './styles.css';

// WebKit's page menu has Reload and Back, which restart the app and drop unsaved drafts.
// Text fields and selected text keep their menu (Copy, Paste, Look Up).
if (import.meta.env.PROD) {
  document.addEventListener('contextmenu', (e) => {
    const t = e.target as Element;
    if (t.closest('input, textarea, [contenteditable="true"]') || window.getSelection()?.toString()) return;
    e.preventDefault();
  });
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
