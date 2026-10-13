import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import "@/packages/ui/styles/index.css";
import "@/brand/theme.css";
import "./globals.css";

export const metadata: Metadata = {
  title: "Brakets — Tournament Bracket UI",
  description: "High-performance bracket board UI for esports and gaming tournaments.",
};

export const viewport: Viewport = {
  themeColor: "#060606",
  colorScheme: "dark",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" data-theme="dark">
      <body className="antialiased bg-[#060606] text-white min-h-screen">
        {children}
      </body>
    </html>
  );
}
