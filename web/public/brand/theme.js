// Apply the existing preference before the first paint and React session check.
(() => {
  let theme = 'system';
  try { theme = localStorage.getItem('nusa_theme') || 'system'; } catch { /* Storage may be unavailable. */ }
  const root = document.documentElement;
  root.classList.toggle('dark', theme === 'dark' || (theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches));
  root.classList.add('theme-ready');
})();
