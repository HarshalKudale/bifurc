/** @type {import('tailwindcss').Config} */
export default {
  content: [
    // The Electron host's entry. It holds `index.html` and `main.tsx` and nothing else — the UI itself
    // is in `packages/gui`, so this glob is kept for the entry's own markup.
    "./renderer/**/*.{html,tsx,ts}",
    // The shared UI (D18) and every package's own sources.
    "./packages/**/*.{html,tsx,ts}",
    // The browser host's container. Its connection form is deliberately styled with the same tokens as
    // the UI it wraps, so its classes have to be scanned too — without this the form renders unstyled,
    // because Tailwind purges anything it cannot see.
    "./web/**/*.{html,tsx,ts}",
  ],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        background:         "oklch(var(--background) / <alpha-value>)",
        surface:            "oklch(var(--surface) / <alpha-value>)",
        card:               "oklch(var(--card) / <alpha-value>)",
        "surface-2":        "oklch(var(--surface-2) / <alpha-value>)",
        secondary:          "oklch(var(--secondary) / <alpha-value>)",
        accent:             "oklch(var(--accent) / <alpha-value>)",
        border:             "oklch(var(--border) / <alpha-value>)",
        foreground:         "oklch(var(--foreground) / <alpha-value>)",
        "muted-foreground": "oklch(var(--muted-foreground) / <alpha-value>)",
        "subtle-text":      "oklch(var(--subtle-text) / <alpha-value>)",
        "primary-foreground":"oklch(var(--primary-foreground) / <alpha-value>)",
        signal:             "oklch(var(--signal) / <alpha-value>)",
        "signal-foreground":"oklch(var(--signal-foreground) / <alpha-value>)",
        violet:             "oklch(var(--violet) / <alpha-value>)",
        amber:              "oklch(var(--amber) / <alpha-value>)",
        blue:               "oklch(var(--blue) / <alpha-value>)",
        destructive:        "oklch(var(--destructive) / <alpha-value>)",
      },
      fontFamily: {
        sans: ["Space Grotesk", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono", "ui-monospace", "SFMono-Regular", "monospace"],
      },
      borderRadius: {
        sm: "var(--radius-sm)",
        md: "var(--radius-md)",
        lg: "var(--radius-lg)",
        xl: "var(--radius-xl)",
      },
    },
  },
  plugins: [],
};
