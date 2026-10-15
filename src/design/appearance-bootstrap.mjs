(function bootstrapAppearance(value, theme) { // eslint-disable-line @typescript-eslint/no-unused-expressions -- Embedded and invoked by the native window bootstrap.
  window.__SAYWEAVE_APPEARANCE__ = value;
  const apply = () => {
    const root = document.documentElement;
    if (!root) return false;
    // The application owns subsequent choices. A delayed bootstrap must not
    // overwrite a theme already applied by the page or a newer native read.
    if (root.dataset.theme) return true;
    for (const [key, color] of Object.entries(theme.vars)) root.style.setProperty(key, color);
    root.dataset.theme = theme.id;
    root.style.colorScheme = theme.scheme;
    return true;
  };
  if (!apply()) document.addEventListener('DOMContentLoaded', apply, {once: true});
})
