// Agenium's element picker, injected into the browser's pages (ADR-121). It has no access to Agenium: a pick leaves the
// page as a navigation to http://agenium-pick.invalid/?p=<json>, which the app cancels and reads.
// The app turns picking on and off by calling window.__ageniumPicker.set(true|false).
// While picking: a click picks the element under the mouse; dragging picks a region (a blank area, or several
// elements at once), with the elements it holds.
(() => {
  if (window.__ageniumPicker) return;
  let on = false;
  let box = null;
  let tag = null;
  let drag = null; // { x, y } where the mouse went down
  let dragged = false;
  const MAX_HTML = 1500;
  const MAX_TEXT = 300;
  const DRAG_MIN = 6;
  const STYLES = ["display", "position", "width", "height", "margin", "padding", "color", "background-color", "font-size", "font-weight", "line-height", "border", "border-radius", "gap", "justify-content", "align-items"];

  function overlay() {
    if (box) return;
    box = document.createElement("div");
    box.style.cssText = "position:fixed;pointer-events:none;z-index:2147483647;border:2px solid #3b82f6;background:rgba(59,130,246,.12);border-radius:3px;display:none";
    tag = document.createElement("div");
    tag.style.cssText = "position:fixed;pointer-events:none;z-index:2147483647;background:#1d4ed8;color:#fff;font:11px/1.4 ui-monospace,monospace;padding:1px 6px;border-radius:3px;display:none;white-space:nowrap";
    document.documentElement.appendChild(box);
    document.documentElement.appendChild(tag);
  }
  function hide() {
    if (box) box.style.display = "none";
    if (tag) tag.style.display = "none";
  }
  function show(l, t, w, h, label, dashed) {
    overlay();
    Object.assign(box.style, { display: "block", left: l + "px", top: t + "px", width: w + "px", height: h + "px", borderStyle: dashed ? "dashed" : "solid" });
    tag.textContent = label;
    Object.assign(tag.style, { display: "block", left: Math.max(0, l) + "px", top: Math.max(0, t - 20) + "px" });
  }
  function short(el) {
    let s = el.tagName.toLowerCase();
    if (el.id) s += "#" + el.id;
    const cls = [...el.classList].slice(0, 2);
    if (cls.length) s += "." + cls.join(".");
    return s;
  }
  function selector(el) {
    const parts = [];
    for (let e = el; e && e.nodeType === 1 && parts.length < 7; e = e.parentElement) {
      if (e.id) {
        parts.unshift("#" + CSS.escape(e.id));
        break;
      }
      let s = e.tagName.toLowerCase();
      const cls = [...e.classList].filter((c) => !/^(hover|active|focus)/.test(c)).slice(0, 2);
      if (cls.length) s += "." + cls.map((c) => CSS.escape(c)).join(".");
      const p = e.parentElement;
      if (p) {
        const same = [...p.children].filter((c) => c.tagName === e.tagName);
        if (same.length > 1) s += ":nth-of-type(" + (same.indexOf(e) + 1) + ")";
      }
      parts.unshift(s);
      if (e.tagName === "BODY") break;
    }
    return parts.join(" > ");
  }
  // The UI component that rendered the element, when the page is a development build that says so.
  function component(el) {
    try {
      const fk = Object.keys(el).find((k) => k.startsWith("__reactFiber$"));
      if (fk) {
        for (let f = el[fk]; f; f = f.return) {
          const t = f.type;
          if (t && typeof t !== "string") {
            const name = t.displayName || t.name;
            const src = f._debugSource || (f._debugOwner && f._debugOwner._debugSource);
            if (name) return { framework: "react", name, file: src ? src.fileName + ":" + src.lineNumber : null };
          }
        }
      }
      const vue = el.__vueParentComponent;
      if (vue && vue.type) return { framework: "vue", name: vue.type.name || vue.type.__name || null, file: vue.type.__file || null };
      const sv = el.__svelte_meta;
      if (sv && sv.loc) return { framework: "svelte", name: null, file: sv.loc.file + ":" + sv.loc.line };
    } catch (_) {}
    return null;
  }
  function textOf(el) {
    return (el.innerText || el.textContent || "").trim().replace(/\s+/g, " ").slice(0, MAX_TEXT);
  }
  function page() {
    return { url: location.href, title: document.title, viewport: [innerWidth, innerHeight], dpr: devicePixelRatio || 1 };
  }
  function describe(el) {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    const styles = {};
    for (const k of STYLES) {
      const v = cs.getPropertyValue(k);
      if (v && v !== "none" && v !== "normal" && v !== "auto" && v !== "0px" && v !== "rgba(0, 0, 0, 0)") styles[k] = v;
    }
    const attrs = {};
    for (const a of el.attributes) {
      if (a.name === "class" || a.name === "style") continue;
      if (/^(id|role|name|type|href|src|alt|title|placeholder|for|aria-|data-)/.test(a.name)) attrs[a.name] = a.value.slice(0, 200);
    }
    const html = el.outerHTML;
    return Object.assign(page(), {
      kind: "element",
      selector: selector(el),
      element: short(el),
      tag: el.tagName.toLowerCase(),
      classes: [...el.classList].slice(0, 12),
      attrs,
      text: textOf(el),
      html: html.length > MAX_HTML ? html.slice(0, MAX_HTML) + "…" : html,
      rect: [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)],
      styles,
      component: component(el),
      parent: el.parentElement ? short(el.parentElement) : null,
    });
  }
  // A dragged region: the outermost visible elements lying in it (several at once), or, for a blank area, the
  // element that holds it.
  function region(l, t, w, h) {
    const inside = (r) => r.width > 0 && r.height > 0 && r.left >= l - 1 && r.top >= t - 1 && r.right <= l + w + 1 && r.bottom <= t + h + 1;
    const all = [...document.body.querySelectorAll("*")].filter((e) => e !== box && e !== tag && inside(e.getBoundingClientRect()));
    const top = all.filter((e) => !all.includes(e.parentElement)).slice(0, 12);
    const holder = document.elementFromPoint(l + w / 2, t + h / 2);
    const html = top.map((e) => e.outerHTML).join("\n");
    return Object.assign(page(), {
      kind: "region",
      selector: holder ? selector(holder) : "body",
      element: top.length ? `${top.length} element(s) in a region` : `blank area in ${holder ? short(holder) : "body"}`,
      tag: "region",
      classes: [],
      attrs: {},
      text: top.map(textOf).filter(Boolean).join(" | ").slice(0, MAX_TEXT),
      html: html.length > MAX_HTML ? html.slice(0, MAX_HTML) + "…" : html,
      rect: [Math.round(l), Math.round(t), Math.round(w), Math.round(h)],
      styles: {},
      component: top[0] ? component(top[0]) : null,
      parent: holder ? short(holder) : null,
      elements: top.map((e) => ({ element: short(e), selector: selector(e), text: textOf(e).slice(0, 80) })),
    });
  }
  function send(p) {
    // The highlight goes before the app takes its screenshot.
    hide();
    location.href = "http://agenium-pick.invalid/?p=" + encodeURIComponent(JSON.stringify(p));
  }
  function onMove(e) {
    if (!on) return;
    if (drag) {
      const l = Math.min(drag.x, e.clientX), t = Math.min(drag.y, e.clientY);
      const w = Math.abs(e.clientX - drag.x), h = Math.abs(e.clientY - drag.y);
      if (w > DRAG_MIN || h > DRAG_MIN) dragged = true;
      if (dragged) show(l, t, w, h, Math.round(w) + "×" + Math.round(h), true);
      return;
    }
    const el = e.target;
    if (!el || el === box || el === tag) return;
    const r = el.getBoundingClientRect();
    show(r.left, r.top, r.width, r.height, short(el) + "  " + Math.round(r.width) + "×" + Math.round(r.height), false);
  }
  function onDown(e) {
    if (!on || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    drag = { x: e.clientX, y: e.clientY };
    dragged = false;
  }
  function onUp(e) {
    if (!on || !drag) return;
    e.preventDefault();
    e.stopPropagation();
    const start = drag;
    drag = null;
    if (dragged) {
      const l = Math.min(start.x, e.clientX), t = Math.min(start.y, e.clientY);
      send(region(l, t, Math.abs(e.clientX - start.x), Math.abs(e.clientY - start.y)));
    } else {
      send(describe(e.target));
    }
  }
  function swallow(e) {
    if (on) {
      e.preventDefault();
      e.stopPropagation();
      e.stopImmediatePropagation();
    }
  }
  // Pointer events drive the picker: cancelling pointerdown would stop the mouse events that follow it.
  addEventListener("pointermove", onMove, true);
  addEventListener("pointerdown", onDown, true);
  addEventListener("pointerup", onUp, true);
  for (const t of ["mousedown", "mouseup", "click", "dblclick", "submit", "dragstart", "selectstart"]) addEventListener(t, swallow, true);
  addEventListener(
    "keydown",
    (e) => {
      if (on && e.key === "Escape") {
        on = false;
        drag = null;
        hide();
        document.documentElement.style.cursor = "";
        location.href = "http://agenium-pick.invalid/?off=1";
      }
    },
    true,
  );
  window.__ageniumPicker = {
    set(v) {
      on = !!v;
      drag = null;
      document.documentElement.style.cursor = on ? "crosshair" : "";
      if (!on) hide();
    },
  };
})();
