// cloudflare/src/models.ts
// Workers AI and Fallback Model Integration

import type { Env } from "./types";

export interface ModelMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

export interface ModelOptions {
  model?: string;
  temperature?: number;
  max_tokens?: number;
}

export async function callModel(
  env: Env,
  messages: ModelMessage[] | string,
  options?: ModelOptions
): Promise<string> {
  const normMessages: ModelMessage[] = typeof messages === "string"
    ? [{ role: "user", content: messages }]
    : messages;

  // Option 1: Custom Ollama endpoint if configured in env
  if (env.OLLAMA_BASE_URL) {
    try {
      const res = await fetch(`${env.OLLAMA_BASE_URL.replace(/\/$/, '')}/api/chat`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: options?.model || "llama3.2",
          messages: normMessages.map(m => ({ role: m.role, content: m.content })),
          stream: false,
          options: {
            temperature: options?.temperature ?? 0.2,
          }
        }),
      });
      if (res.ok) {
        const data: any = await res.json();
        return data.message?.content || "";
      }
    } catch (e) {
      console.warn("Ollama call failed, falling back to Workers AI:", e);
    }
  }

  // Option 2: Workers AI (Coding model: Meta Llama 3.3 70B Instruct or Llama 3.1 8B Instruct)
  if (!env.AI) {
    throw new Error("Neither Workers AI (env.AI) nor OLLAMA_BASE_URL is available in environment");
  }

  const model = options?.model || "@cf/meta/llama-3.3-70b-instruct-fp8-fast";
  try {
    const aiRes: any = await env.AI.run(model, {
      messages: normMessages.map(m => ({ role: m.role, content: m.content })),
      temperature: options?.temperature ?? 0.2,
      max_tokens: options?.max_tokens ?? 2048,
    });
    return aiRes.response || aiRes.content || (typeof aiRes === "string" ? aiRes : JSON.stringify(aiRes));
  } catch (err: any) {
    // Graceful fallback to Qwen2.5-Coder or Llama-3.1-8B-FP8
    try {
      const fallbackRes: any = await env.AI.run("@cf/qwen/qwen2.5-coder-32b-instruct", {
        messages: normMessages.map(m => ({ role: m.role, content: m.content })),
        temperature: options?.temperature ?? 0.2,
        max_tokens: options?.max_tokens ?? 2048,
      });
      return fallbackRes.response || fallbackRes.content || "";
    } catch (fallbackErr) {
      try {
        const fallbackRes2: any = await env.AI.run("@cf/meta/llama-3.1-8b-instruct-fp8", {
          messages: normMessages.map(m => ({ role: m.role, content: m.content })),
          temperature: options?.temperature ?? 0.2,
          max_tokens: options?.max_tokens ?? 2048,
        });
        return fallbackRes2.response || fallbackRes2.content || "";
      } catch (e) {
        throw err;
      }
    }
  }
}
