import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        "action-blue": "#0066cc",
        "action-blue-focus": "#0071e3",
        "action-blue-dark": "#2997ff",
        ink: "#1d1d1f",
        "ink-muted-80": "#333333",
        "ink-muted-48": "#7a7a7a",
        "ink-muted-12": "#1d1d1f1f",
        parchment: "#f5f5f7",
        pearl: "#fafafc",
        "tile-1": "#272729",
        "tile-2": "#2a2a2c",
        "tile-3": "#252527",
        hairline: "#e0e0e0",
        "divider-soft": "#f0f0f0",
      },
      fontFamily: {
        display: [
          "SF Pro Display",
          "system-ui",
          "-apple-system",
          "BlinkMacSystemFont",
          "PingFang SC",
          "Microsoft YaHei",
          "sans-serif",
        ],
        text: [
          "SF Pro Text",
          "system-ui",
          "-apple-system",
          "BlinkMacSystemFont",
          "PingFang SC",
          "Microsoft YaHei",
          "sans-serif",
        ],
      },
      fontSize: {
        hero: ["56px", { lineHeight: "1.07", letterSpacing: "-0.28px" }],
        "display-lg": ["40px", { lineHeight: "1.1", letterSpacing: "0" }],
        "display-md": ["34px", { lineHeight: "1.15", letterSpacing: "-0.374px" }],
        lead: ["28px", { lineHeight: "1.14", letterSpacing: "0.196px" }],
        tagline: ["21px", { lineHeight: "1.19", letterSpacing: "0.231px" }],
        body: ["17px", { lineHeight: "1.47", letterSpacing: "-0.374px" }],
        caption: ["14px", { lineHeight: "1.43", letterSpacing: "-0.224px" }],
        fine: ["12px", { lineHeight: "1.0", letterSpacing: "-0.12px" }],
      },
      borderRadius: {
        xs: "5px",
        sm: "8px",
        md: "11px",
        lg: "18px",
        pill: "9999px",
      },
      boxShadow: {
        product: "0 5px 30px 3px rgba(0,0,0,0.22)",
        hairline: "0 0 0 1px #e0e0e0",
      },
    },
  },
  plugins: [],
} satisfies Config;
