/**
 * Brakets Tailwind preset.
 * Reusable across apps and packages/ui. Mirrors brand/design-tokens.json and brand/theme.css.
 */

/** @type {import('tailwindcss').Config} */
const preset = {
  theme: {
    extend: {
      colors: {
        // Raw brand palette
        brakets: {
          teal: "#3ab6a1",
          blue: "#4599ca",
          gray: "#5f6264",
          white: "#ffffff",
          black: "#060606",
        },
        // Semantic aliases
        primary: "#3ab6a1",
        secondary: "#4599ca",
        muted: "#5f6264",
        ink: "#060606",
      },
      backgroundImage: {
        "brand-gradient": "linear-gradient(135deg, #3ab6a1 0%, #4599ca 100%)",
      },
    },
  },
};

module.exports = preset;
