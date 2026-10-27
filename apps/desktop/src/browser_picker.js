// Agenium's element picker, injected into the preview browser (ADR-121). It has no access to Agenium: a picked
// element leaves the page as a navigation to http://agenium-pick.invalid/?p=<json>, which the app cancels and reads.
// The app turns picking on and off by calling window.__ageniumPicker.set(true|false).
(() => {
  if (window.__ageniumPicker) return;
  let on = false;
  let box = null;
  let tag = null;
  const MAX_HTML = 1500;
  const MAX_TEXT = 300;
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
    return {
      url: location.href,
      title: document.title,
      viewport: [innerWidth, innerHeight],
      selector: selector(el),
      element: short(el),
      tag: el.tagName.toLowerCase(),
      classes: [...el.classList].slice(0, 12),
      attrs,
      text: (el.innerText || el.textContent || "").trim().replace(/\s+/g, " ").slice(0, MAX_TEXT),
      html: html.length > MAX_HTML ? html.slice(0, MAX_HTML) + "…" : html,
      rect: [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)],
      styles,
      component: component(el),
      parent: el.parentElement ? short(el.parentElement) : null,
    };
  }
  function onMove(e) {
    if (!on) return;
    const el = e.target;
    if (!el || el === box || el === tag) return;
    overlay();
    const r = el.getBoundingClientRect();
    Object.assign(box.style, { display: "block", left: r.left + "px", top: r.top + "px", width: r.width + "px", height: r.height + "px" });
    tag.textContent = short(el) + "  " + Math.round(r.width) + "×" + Math.round(r.height);
    Object.assign(tag.style, { display: "block", left: Math.max(0, r.left) + "px", top: Math.max(0, r.top - 20) + "px" });
  }
  function onClick(e) {
    if (!on) return;
    e.preventDefault();
    e.stopPropagation();
    e.stopImmediatePropagation();
    const p = describe(e.target);
    location.href = "http://agenium-pick.invalid/?p=" + encodeURIComponent(JSON.stringify(p));
  }
  function swallow(e) {
    if (on) {
      e.preventDefault();
      e.stopPropagation();
    }
  }
  addEventListener("mousemove", onMove, true);
  addEventListener("click", onClick, true);
  for (const t of ["mousedown", "mouseup", "pointerdown", "pointerup", "submit"]) addEventListener(t, swallow, true);
  addEventListener(
    "keydown",
    (e) => {
      if (on && e.key === "Escape") {
        on = false;
        hide();
        location.href = "http://agenium-pick.invalid/?off=1";
      }
    },
    true,
  );
  window.__ageniumPicker = {
    set(v) {
      on = !!v;
      document.documentElement.style.cursor = on ? "crosshair" : "";
      if (!on) hide();
    },
  };
})();
