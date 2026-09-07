Local Panel Design System
Astetics - Cyberpunk Neon Developer Tool

A dark, technical interface with phosphor-green signal accents, restrained glows, grid details, and precise developer-tool UI. It should feel like a capable desktop utility, not a gaming interface: dense when needed, readable always, and neon only where attention is required.

---

1. Design principles

1. Signal, not decoration  
   Neon green represents an actionable, active, successful, connected, or selected state. Never use it as a general-purpose background color.

2. Dark surfaces create hierarchy  
   Use subtle blue-charcoal surface steps, borders, and spacing to establish depth. Avoid large shadows and glassmorphism.

3. Technical, calm, and precise  
   Interface copy is concise. Use monospaced type for URLs, ports, commands, request methods, variables, and system states.

4. Glow is earned  
   Glows identify live signal, interaction, or system activity. Most cards and text should not glow.

---

2. Color palette

The system uses OKLCH tokens so colors behave predictably across displays and can be used directly in CSS.

Core surfaces

| Token | Value | Use |
|---|---:|---|
| `--background` | `oklch(0.16 0.012 250)` | Page background |
| `--surface` | `oklch(0.19 0.013 252)` | Panels, navigation, dark product UI |
| `--card` | `oklch(0.20 0.014 252)` | Cards, modals, elevated containers |
| `--surface-2` | `oklch(0.23 0.015 252)` | Hovered panels, grouped controls |
| `--secondary` | `oklch(0.26 0.016 252)` | Secondary button and selected background |
| `--accent` | `oklch(0.28 0.020 252)` | Active row, input focus background |
| `--border` | `oklch(0.31 0.016 252)` | Default border, dividers, grid lines |

Rule: surfaces should be blue-charcoal rather than pure black. This retains depth, makes borders readable, and supports the green signal color.

Text

| Token | Value | Use |
|---|---:|---|
| `--foreground` | `oklch(0.96 0.005 250)` | Main headings and body copy |
| `--muted-foreground` | `oklch(0.68 0.018 252)` | Supporting copy, metadata, labels |
| `--subtle-text` | `oklch(0.52 0.016 252)` | Disabled or tertiary information |
| `--primary-foreground` | `oklch(0.18 0.020 190)` | Text on neon green buttons |

Use the full foreground color sparingly in dense UI. Supporting descriptions should default to muted text.

Signal and semantic colors

| Token | Value | Meaning |
|---|---:|---|
| `--signal` | `oklch(0.84 0.16 172)` | Brand green, primary action, online/success |
| `--signal-foreground` | `oklch(0.18 0.02 190)` | Dark text placed on the signal fill |
| `--violet` | `oklch(0.72 0.16 300)` | WebSocket, special system state, secondary chart series |
| `--amber` | `oklch(0.83 0.15 78)` | Warnings, pending actions, HTTP redirects |
| `--blue` | `oklch(0.66 0.15 240)` | Informational status, REST/API metadata |
| `--destructive` | `oklch(0.63 0.21 22)` | Errors, destructive actions, failed requests |

Suggested protocol mapping

Use protocol colors consistently in product UI:

| Type | Color |
|---|---|
| Connected / successful / `2xx` | Signal green |
| Redirect / `3xx` | Amber |
| Client or server error / `4xx`, `5xx` | Destructive red |
| REST / informational | Blue |
| WebSocket / live connection | Violet |
| HTTPS / secure traffic | Signal green, optionally with muted blue support |

Do not rely on color alone. Pair it with a label, icon, status dot, or code.

---

3. Typography

Font pairing

| Role | Typeface | Fallback |
|---|---|---|
| Interface, marketing, headings | Space Grotesk | `ui-sans-serif, system-ui, sans-serif` |
| Code, API data, command line | JetBrains Mono | `ui-monospace, SFMono-Regular, monospace` |

Type scale

| Style | Size / line height | Typical use |
|---|---|---|
| Display | `64px / 1.02` | Hero heading on desktop |
| H1 | `48px / 1.08` | Page heading |
| H2 | `36px / 1.15` | Main section heading |
| H3 | `24px / 1.25` | Card and subsection title |
| H4 | `18px / 1.35` | Compact card title |
| Body large | `18px / 1.6` | Hero supporting copy |
| Body | `16px / 1.6` | Default marketing and app copy |
| Body compact | `14px / 1.5` | Product UI descriptions |
| Label | `12px / 1.35` | Input labels, metadata, eyebrow text |
| Code compact | `12px / 1.5` | URLs, headers, payload values, terminal lines |

Typography rules

- Use Space Grotesk for all readable interface and marketing copy.
- Use JetBrains Mono only for technical content: domains, endpoints, ports, commands, environment variables, request methods, timestamps, and status codes.
- Hero headlines should be direct and compact, usually no more than 2–3 lines.
- Avoid excessive all-caps. If using it for an eyebrow label, keep it to short metadata such as `Local API infrastructure`.
- Prefer sentence case for buttons, navigation, labels, and body copy.
- Never use the neon accent for long text passages. Use it for an emphasized word, compact label, status, or CTA.

---

4. Spacing and layout

Base unit

Use a 4px base spacing unit. All gaps, padding, and layout measurements should be multiples of 4.

| Token | Value | Use |
|---|---:|---|
| `space-1` | `4px` | Icon-to-text gap, micro spacing |
| `space-2` | `8px` | Compact controls, stacked labels |
| `space-3` | `12px` | Input padding, compact cards |
| `space-4` | `16px` | Default component gap |
| `space-5` | `20px` | Card padding in dense UI |
| `space-6` | `24px` | Standard card padding |
| `space-8` | `32px` | Component groups |
| `space-10` | `40px` | Larger module separation |
| `space-12` | `48px` | Section subgroups |
| `space-16` | `64px` | Standard vertical section space |
| `space-20` | `80px` | Landing-page section space |
| `space-24` | `96px` | Major page separation |
| `space-32` | `128px` | Hero breathing room |

Container and grid

- Desktop max content width: `1200px` to `1280px`
- Readable text width: `640px` to `720px`
- Desktop page padding: `32px` to `48px`
- Tablet page padding: `24px`
- Mobile page padding: `16px` to `20px`
- Use a 12-column grid for desktop pages.
- Use an 8px grid backdrop only on major marketing sections or product illustrations, never underneath dense reading content.

Section rhythm

For marketing pages:

- Eyebrow to heading: `16px`
- Heading to supporting paragraph: `20px`
- Supporting paragraph to CTA group: `32px`
- CTA group to product visual: `48px–64px`
- Major landing sections: `96px–128px` vertical padding on desktop
- Reduce major sections to `64px–80px` on mobile

---

5. Shape, borders, and surfaces

Radius

| Token | Value | Use |
|---|---:|---|
| `--radius-sm` | `4px` | Tags, compact controls, code chips |
| `--radius-md` | `6px` | Inputs, buttons, table rows |
| `--radius-lg` | `8px` | Cards, panels, product mockups |
| `--radius-xl` | `12px` | Large feature cards and dialogs |

Avoid pill-heavy interfaces. Use full pills only for statuses, short tags, and compact filters.

Borders

- Default border: `1px solid var(--border)`
- Low-emphasis divider: use the border color at reduced opacity.
- Active border: green signal at roughly 50–70% opacity.
- Hovered card: lighten the border slightly before adding glow.
- Use borders to separate surfaces. Avoid relying on drop shadows for hierarchy.

Surface hierarchy

1. Background: the page canvas  
2. Surface: sidebars, large embedded UI regions  
3. Card: standard content cards and product panes  
4. Surface 2: hover, selected, or grouped-control backgrounds  
5. Accent surface: minimal use for selected states or callouts  

Never create more than these five visual elevation levels in one screen.

---

6. Glow rules

The signature visual detail is a soft phosphor-green glow. It should feel like a signal emitted by the interface, not a generic shadow.

Green glow tokens

```css
--glow-signal-sm: 0 0 12px color-mix(in oklab, var(--signal) 22%, transparent);
--glow-signal-md: 0 0 24px color-mix(in oklab, var(--signal) 26%, transparent);
--glow-signal-lg: 0 0 56px color-mix(in oklab, var(--signal) 20%, transparent);

--glow-violet-sm: 0 0 16px color-mix(in oklab, var(--violet) 22%, transparent);
--glow-danger-sm: 0 0 16px color-mix(in oklab, var(--destructive) 18%, transparent);
```

When to use glow

| Element | Glow level |
|---|---|
| Small live status dot | Small |
| Focused input or selected segmented control | Small |
| Primary CTA hover | Small to medium |
| Active network route or request-flow node | Medium |
| Hero product visual / ambient backdrop | Large, very low opacity |
| Error or warning status | Small semantic glow only |

Glow restrictions

- Do not apply glow to every card, button, heading, icon, and border at once.
- On a single visual viewport, use one main glow focal point and no more than two secondary signal glows.
- Keep blur broad and low-opacity. Avoid hard, saturated outer glows.
- Glows must never reduce text contrast or make technical UI harder to scan.
- For reduced-motion users, keep static glows subtle and disable pulsing.

Example CSS

```css
.signal-status {
  background: var(--signal);
  box-shadow: var(--glow-signal-sm);
}

.primary-button:hover {
  box-shadow: var(--glow-signal-md);
}

.hero-ambient-glow {
  background: color-mix(in oklab, var(--signal) 12%, transparent);
  filter: blur(72px);
  opacity: 0.55;
}
```

---

7. Components

Buttons

Primary
- Signal-green fill
- Dark signal foreground text
- `6px–8px` radius
- Minimum height: `40px`
- Use for one primary action per section, such as Download on GitHub or Get started
- Hover: slightly brighter fill and medium green glow
- Focus: visible green outline plus small glow

Secondary
- Transparent or `--secondary` background
- `--border` border
- Foreground text
- Hover: surface lift and brighter border
- No persistent glow

Ghost
- Transparent background
- Muted text by default, foreground on hover
- Use for navigation and low-priority actions

Destructive
- Use red only for irreversible actions
- Never use destructive styling for standard cancellation

Inputs

- `40px–44px` height
- Surface or card background
- 1px default border
- `12px–16px` horizontal padding
- Label above input in `12px` interface type
- Focus state: green border and restrained small green glow
- Error state: destructive border, error message, no green glow

Cards

- Surface: `--card`
- Radius: `8px–12px`
- Border: 1px default border
- Default padding: `24px`
- Dense product panels: `16px–20px`
- Hover only if interactive: subtle border lift, optional 1–2px upward translation
- Decorative grids or corner markers should be low contrast and never compete with content

Status badges

Use small, compact tags with mono text:

- Green: `Live`, `Connected`, `200 OK`
- Violet: `WebSocket`, `Streaming`
- Amber: `Warning`, `302 Redirect`
- Red: `Failed`, `500 Error`
- Neutral: `Draft`, `Local`, `Disabled`

Badge treatment should combine:
- low-opacity semantic background
- semantic border
- readable semantic text
- optional 6px status dot

---

8. Iconography and illustration

Icons

- Use simple outlined icons with a consistent `1.5px–2px` stroke.
- Default icon color: muted foreground.
- Active icon color: signal green.
- Do not use more than one accent color in the same compact control.
- Keep icon containers square or softly rounded, not circular by default.

Product imagery

The visual language should show the product as a working local network control panel:

- Browser domains such as `app.localhost`
- Port routes such as `:3000`, `:5173`, `:8080`
- Request and response flows
- HTTP method badges: `GET`, `POST`, `PATCH`, `DELETE`
- Traffic streams, timestamps, response codes, WebSocket events
- Terminal-like command snippets
- Connection nodes with sparse green activity lines

Use mockups that look structurally believable. Avoid generic floating dashboard cards with meaningless charts.

Background treatments

- Fine grid on dark canvas
- Subtle radial green ambient light behind the hero mockup
- Faint scan-line or technical coordinate details only at very low opacity
- Keep backgrounds quiet enough for white text to remain dominant

---

9. Motion and interaction

Motion should reinforce routing, traffic, and system responsiveness.

Motion timing

| Interaction | Duration | Easing |
|---|---:|---|
| Hover state | `150ms–200ms` | ease-out |
| Button press | `100ms–150ms` | ease-in-out |
| Card reveal | `300ms–450ms` | ease-out |
| Section entrance | `500ms–700ms` | ease-out |
| Data flow / traffic pulse | `1.6s–2.4s` looping | linear or ease-in-out |

Recommended animations

- Traffic pulse: a small green point moving through a route line
- Route activation: border brightens then returns to its base state
- Terminal reveal: lines appear in quick, staggered order
- Hero entrance: copy and product mockup fade and translate up slightly
- Scroll reveal: use once per section, not on every child element

Avoid

- Constant full-page motion
- Fast blinking
- Neon flicker effects on readable text
- Large parallax shifts
- Long animation delays that make the product feel slow

Respect `prefers-reduced-motion`: remove route travel, pulse loops, and large transforms.

---

10. Accessibility and contrast

- Main text must maintain strong contrast against `--background`, `--surface`, and `--card`.
- Use green as a signal, never as the only way to identify a status.
- Focus indicators must be visible with keyboard navigation, using a green outline or ring.
- Avoid small muted text below `12px`.
- Avoid long paragraphs in mono type.
- Never animate error messages or critical system warnings in a way that makes them difficult to read.
- Ensure buttons communicate state through label changes, not color alone, for example: `Connecting…`, `Connected`, `Retry`.

---

11. CSS token starter

```css
:root {
  --radius: 0.5rem;

  --background: oklch(0.16 0.012 250);
  --foreground: oklch(0.96 0.005 250);

  --surface: oklch(0.19 0.013 252);
  --card: oklch(0.20 0.014 252);
  --surface-2: oklch(0.23 0.015 252);
  --secondary: oklch(0.26 0.016 252);
  --accent: oklch(0.28 0.020 252);
  --border: oklch(0.31 0.016 252);

  --muted-foreground: oklch(0.68 0.018 252);
  --subtle-text: oklch(0.52 0.016 252);

  --signal: oklch(0.84 0.16 172);
  --signal-foreground: oklch(0.18 0.020 190);
  --violet: oklch(0.72 0.16 300);
  --amber: oklch(0.83 0.15 78);
  --blue: oklch(0.66 0.15 240);
  --destructive: oklch(0.63 0.21 22);

  --font-sans: "Space Grotesk", ui-sans-serif, system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, SFMono-Regular, monospace;

  --glow-signal-sm: 0 0 12px color-mix(in oklab, var(--signal) 22%, transparent);
  --glow-signal-md: 0 0 24px color-mix(in oklab, var(--signal) 26%, transparent);
  --glow-signal-lg: 0 0 56px color-mix(in oklab, var(--signal) 20%, transparent);
}
```

---

12. Quick implementation checklist

Before calling a new screen or page “on brand,” confirm:

- [ ] Blue-charcoal background and stepped surfaces, not flat black
- [ ] Signal green reserved for primary actions, active states, and successful activity
- [ ] Space Grotesk for UI and marketing, JetBrains Mono for technical data
- [ ] 4px spacing rhythm used consistently
- [ ] Borders establish hierarchy before shadows or glow
- [ ] Only one dominant green glow focal point per viewport
- [ ] Product visuals include believable domains, ports, requests, or traffic
- [ ] Motion depicts system activity rather than decoration
- [ ] Text, focus, and status states remain accessible without color alone
