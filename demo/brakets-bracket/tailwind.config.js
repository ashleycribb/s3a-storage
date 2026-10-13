/** @type {import('tailwindcss').Config} */
const braketsPreset = require("./brand/tailwind-preset.js");

module.exports = {
  presets: [braketsPreset],
  content: [
    "./app/**/*.{js,ts,jsx,tsx,mdx}",
    "./pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./components/**/*.{js,ts,jsx,tsx,mdx}",
    "./packages/ui/src/**/*.{js,ts,jsx,tsx,mdx}",
  ],
  plugins: [],
};
