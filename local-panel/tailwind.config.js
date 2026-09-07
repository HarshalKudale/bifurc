/** @type {import('tailwindcss').Config} */
export default {
  content: ["./renderer/**/*.{html,tsx,ts}"],
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
