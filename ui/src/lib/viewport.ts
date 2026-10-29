/**
 * The window's client area, for placing flyouts. Inside Tauri that is the viewport; in the
 * browser preview it is the framed stand-in window (#app), so menus behave as they will in
 * the real, smaller window.
 */
export function viewportRect(): { top: number; bottom: number; left: number; right: number } {
  const app = document.documentElement.classList.contains('preview') ? document.getElementById('app') : null;
  if (app) {
    const r = app.getBoundingClientRect();
    return { top: r.top, bottom: r.bottom, left: r.left, right: r.right };
  }
  return { top: 0, bottom: window.innerHeight, left: 0, right: window.innerWidth };
}
