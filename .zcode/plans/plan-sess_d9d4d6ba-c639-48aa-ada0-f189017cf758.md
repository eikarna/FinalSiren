## Goal
Redesign SirenWeb from "tacky cyberpunk cyan-neon" to a **clean, elegant light theme** with an **emerald** accent, applied consistently across all 5 pages + all JS-generated DOM. Keep Tailwind via CDN (no build step), add restrained mature animations, and fix the audit-found dead/broken code along the way.

## Design system (the new look)
**Light, airy, professional** (Stripe/Notion/Vercel-docs feel):

| Token | Value | Use |
|---|---|---|
| Page bg | `#fafafa` (zinc-50) + very subtle radial wash | body |
| Card bg | `#ffffff` | main panels, inputs |
| Text primary | `slate-800` (#1e293b) | headings, body |
| Text secondary | `slate-500` (#64748b) | labels, hints |
| Borders | `slate-200` (#e2e8f0), hover `emerald-400` | cards, inputs |
| Accent | `emerald-600` #059669 (buttons, links, focus rings) | |
| Accent soft | `emerald-50` #ecfdf5 (active fills, hovers) | |
| Font | **Inter** (display + body), `ui-monospace`/`mono` for code/latency | replaces Orbitron/Rajdhani/Share Tech Mono |
| Radii | `rounded-xl` cards, `rounded-lg` inputs/buttons (consistent) | |
| Shadows | `shadow-sm` default, `shadow-md hover` (NO colored glow) | |

**Removed (the tacky stuff):** Orbitron/Rajdhani/Share Tech Mono fonts; all `bg-clip-text text-transparent` gradient text; cyan→teal gradients on buttons/bars/headings; `hover:shadow-cyan-500/50` neon glows; `animate-pulse` FAB; ALL-CAPS `tracking-widest` eyebrows; the 4 competing border radii; the cyan checker "checking" state (→ emerald).

**Animations (CSS-only + optional GSAP):**
- CSS: page-load fade-in-up (staggered), card hover lift (`hover:-translate-y-0.5` + soft shadow), button press scale, spinner. All gated behind `@media (prefers-reduced-motion: reduce)`.
- GSAP via CDN (`gsap@3` — mature, MIT, free): staggered entrance on the home nav grid + result lists. Used sparingly. If it adds risk, CSS covers the same ground.

## How consistency is enforced
1. **Inline `tailwind.config`** on every page defining `theme.extend.colors.brand` + `fontFamily.sans: Inter` + `boxShadow.soft` → so `bg-brand-600`, `font-sans`, `shadow-soft` work as utilities everywhere (including JS-generated DOM).
2. **Shared partial injection** — a new `js/layout.js` that builds the `<head>` tokens, footer, donation FAB + modal via `document.write`/injection at a `#layout-mount` placeholder. Eliminates the 5× hand-copied chrome (the biggest source of drift). `common.js` keeps the modal logic.
3. **CSS variables** in a tiny `theme.css` (`--brand`, `--brand-soft`, `--surface`, `--text`, `--muted`, `--border`) for any inline-style code (toasts) so it stays in sync with Tailwind.

## File-by-file changes

### New files
- **`css/theme.css`** — CSS variables + base resets + keyframes (fade-in-up, reduced-motion guard) + a few component classes (`.btn-primary`, `.card`, `.input`). Loaded once per page. ~60 lines.
- **`js/layout.js`** — injects the shared footer/FAB/modal + sets `lucide.createIcons()` after injection. Replaces ~60 duplicated lines × 5 pages.
- **`favicon.svg`** (or inline data-URI) — a real brand mark (emerald "N" monogram in rounded square) replacing the generic Material "public" placeholder.

### Edited — all 5 HTML pages (index/sub/link/converter/check)
- New `<head>`: swap fonts → Inter (single family); add `<script>tailwind.config = {...}</script>` with brand tokens; load `css/theme.css`; replace favicon; load GSAP (home only) + `js/layout.js`. Remove dead `og:image` (converter), dead js-yaml (converter), dead qrcode (link).
- New `<body>`: light bg; white cards with slate borders; emerald accents; Inter font. Replace the duplicated footer/FAB/modal with a `<div id="layout-mount"></div>` placeholder.
- Per page: rewrite all the cyan/slate utility classes (full inventory already mapped: converter ~L30-173, link ~L26-253, the others similar).

### Edited — JS (dynamic DOM restyle + bug fixes)
- **`js/common.js`**: toasts → emerald/red/slate via CSS vars; replace broken Font Awesome `<i class="fas">` with inline Lucide SVG paths (or simple unicode). Remove unused `setCurrentYear`/`generateUUID` if dead (keep `copyToClipboard`/`showToast`).
- **`js/check.js`**: result cards → white bg / slate border / emerald hover; status dots keep emerald/rose (semantic); latency badges → emerald-600/amber-600/rose-600 (darker for light bg contrast).
- **`js/link.js`**: proxy cards, status badges, config-link rows → light theme classes; replace FA chevrons with Lucide; remove dead qrcode references.
- **`js/sub.js`**: copy button FA icons → Lucide; validation bar → emerald fill.
- **`js/index.js`**: remove the dead neon particle/menu code (targets non-existent elements); keep only what's wired.
- **`converter.html` inline script — FIX THE BUG**: replace all 43 `dist/lucide.min.js/n` → `\n` (corrupted newline escapes breaking the converter). This is a real functional fix, not cosmetic.
- **Delete `js/converter.js`** (orphan — never loaded by any page).

## Scope confirmed (all selected)
- ✅ Restyle all pages + all dynamic UI
- ✅ Fix dead/broken code (converter 43× corruption, FA toast icons, dead includes, stray og:image, dead index.js, orphan converter.js)
- ✅ Extract shared chrome into `js/layout.js` (5 copies → 1)
- ✅ Add brand favicon

## Deployment
Single repo (SirenWeb), one commit (or two: theme + cleanup). Push to `main`. Worker picks up via raw GitHub URLs automatically (no FinalSiren changes needed).

## Verification
- Open each page locally (or after push): light theme renders, emerald accent, Inter font, no neon, no gradient text.
- Converter: paste a multi-line V2Ray link → splits correctly (the `\n` bug is fixed).
- Link/sub: proxy checks still work (calls unchanged, only classes restyled); status dots/latency colors readable on light bg.
- Donation modal opens/closes (shared layout.js wires it).
- `prefers-reduced-motion`: animations disable cleanly.
- Lighthouse-friendly: no broken icon references, no dead includes.

## Risks & mitigations
| Risk | Mitigation |
|---|---|
| Tailwind CDN `play` script is dev-only (shows console warning) | Acceptable for this project's scale; user chose "keep Tailwind CDN." Note trade-off; offer CLI build as a follow-up. |
| Shared `layout.js` injection breaks if a page lacks `#layout-mount` | Every page gets the placeholder; layout.js no-ops gracefully if absent. |
| GSAP adds a dependency | Mature/MIT/free; used only for home entrance. CSS fallback covers it; guarded by reduced-motion. If it proves flaky, drop it — the design doesn't depend on it. |
| Contrast of latency dots on light bg | Use -600 shades (emerald-600 etc.) not -400. |
| Breaking converter behavior while fixing the `\n` bug | Surgical string replacement only; verify multi-line split after. |

## Files changed
- New: `css/theme.css`, `js/layout.js`, `favicon.svg`
- Edited: `index.html`, `sub.html`, `link.html`, `converter.html`, `check.html`, `js/common.js`, `js/check.js`, `js/link.js`, `js/sub.js`, `js/index.js`
- Deleted: `js/converter.js` (orphan)