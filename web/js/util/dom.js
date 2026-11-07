// Small DOM helpers. All text goes through text nodes / textContent: never innerHTML.

export function $(id) {
  const node = document.getElementById(id);
  if (!node) throw new Error('Missing element #' + id);
  return node;
}

export function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === 'class') node.className = value;
    else if (key.startsWith('on')) node.addEventListener(key.slice(2).toLowerCase(), value);
    else if (value === true) node.setAttribute(key, '');
    else node.setAttribute(key, String(value));
  }
  node.append(...children.flat().filter(child => child !== null && child !== undefined && child !== false));
  return node;
}

export function show(node, visible) {
  node.hidden = !visible;
}
