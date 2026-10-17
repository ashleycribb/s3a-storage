// cloudflare/src/sandbox.ts
// Cloudflare Sandbox Container Integration

import { getSandbox, proxyToSandbox, Sandbox as CloudflareSandbox, ContainerProxy } from "@cloudflare/sandbox";
import type { Env } from "./types";

class S3ASandbox extends CloudflareSandbox<Env> {
  override enableInternet = true;

  async forwardToPort(request: Request, port: number = 3001): Promise<Response> {
    const container = (this as any).ctx?.container;
    if (container) {
      if (!container.running) {
        container.start();
      }
      // Ensure the background server is running inside the container if standalone server exists
      try {
        await this.exec(
          "if [ -f '/workspace/baseline-repo/demo/brakets-bracket/.next/standalone/server.js' ] && ! pgrep -f 'server.js' > /dev/null; then " +
          "cd /workspace/baseline-repo/demo/brakets-bracket/.next/standalone && PORT=3001 HOSTNAME=0.0.0.0 node server.js >/dev/null 2>&1 & " +
          "sleep 0.5; fi"
        );
      } catch (e) {}

      const tcp = container.getTcpPort(port);
      let lastErr: any = null;
      for (let attempt = 0; attempt < 8; attempt++) {
        try {
          return await tcp.fetch(request.clone());
        } catch (err: any) {
          lastErr = err;
          await new Promise((r) => setTimeout(r, 250));
        }
      }
      throw lastErr;
    }
    return (this as any).containerFetch(request, port);
  }
}

(S3ASandbox as any).outbound = async (request: Request) => {
  return fetch(request);
};

export { S3ASandbox as Sandbox, ContainerProxy, getSandbox, proxyToSandbox, CloudflareSandbox };
