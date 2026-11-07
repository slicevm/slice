import http from "node:http";
import { Transform } from "node:stream";
import { pipeline } from "node:stream/promises";

type NextFn = (err?: unknown) => void;
type Middleware = (req: any, res: any, next: NextFn) => void;

function createExpressCompatApp() {
  const middlewares: Middleware[] = [];
  const routes: Array<{ method: string; path: string; handler: Middleware }> = [];

  const app: any = function (req: any, res: any) {
    res.status = function (code: number) {
      res.statusCode = code;
      return res;
    };
    res.json = function (payload: unknown) {
      res.setHeader("Content-Type", "application/json; charset=utf-8");
      res.end(JSON.stringify(payload));
      return res;
    };

    let idx = 0;
    const runNext: NextFn = (err?: unknown) => {
      if (err) {
        res.status(500).json({ error: String((err as Error)?.message || err) });
        return;
      }
      if (idx < middlewares.length) {
        const mw = middlewares[idx++];
        mw(req, res, runNext);
        return;
      }
      const urlPath = String(req.url || "/").split("?")[0];
      for (const r of routes) {
        if (r.method === req.method && r.path === urlPath) {
          r.handler(req, res, runNext);
          return;
        }
      }
      res.status(404).json({ error: "Not Found", path: urlPath });
    };
    runNext();
  };

  app.use = (mw: Middleware) => {
    middlewares.push(mw);
    return app;
  };
  app.get = (path: string, handler: Middleware) => {
    routes.push({ method: "GET", path, handler });
    return app;
  };
  app.post = (path: string, handler: Middleware) => {
    routes.push({ method: "POST", path, handler });
    return app;
  };
  return app;
}

const app = createExpressCompatApp();

app.use((req, res, next) => {
  res.setHeader("X-Powered-By", "Slice-Node-Http");
  const chunks: Buffer[] = [];
  req.on("data", (c: Buffer) => chunks.push(Buffer.isBuffer(c) ? c : Buffer.from(c)));
  req.on("end", () => {
    const raw = Buffer.concat(chunks).toString("utf8");
    req.body = raw ? JSON.parse(raw) : {};
    next();
  });
});

app.get("/health", (_req, res) => {
  res.status(200).json({ status: "healthy:Node-HTTP-Express-Compat", methods: http.METHODS.length });
});

app.post("/api/echo-upper", async (req, res) => {
  const input = String(req.body?.message || "");
  const upperTransform = new Transform({
    transform(chunk, _enc, cb) {
      cb(null, Buffer.from(String(chunk).toUpperCase(), "utf8"));
    }
  });
  res.writeHead(201, { "Content-Type": "text/plain; charset=utf-8" });
  const src = stFromText(input);
  await pipeline(src, upperTransform, res);
});

function stFromText(txt: string) {
  const { Readable } = require("node:stream");
  return Readable.from([Buffer.from(txt, "utf8")]);
}

const server = http.createServer(app);
server.listen(0);
const addr = server.address() as { port: number; address: string };

function requestLocal(method: string, path: string, body?: string): Promise<{ status: number; headers: any; text: string }> {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: "127.0.0.1",
        port: addr.port,
        path,
        method,
        headers: body ? { "Content-Type": "application/json" } : {}
      },
      (res: any) => {
        const chunks: Buffer[] = [];
        res.on("data", (c: Buffer) => chunks.push(Buffer.isBuffer(c) ? c : Buffer.from(c)));
        res.on("end", () => {
          resolve({
            status: res.statusCode,
            headers: res.headers,
            text: Buffer.concat(chunks).toString("utf8")
          });
        });
      }
    );
    req.on("error", reject);
    if (body) req.write(body);
    req.end();
  });
}

(async () => {
  const r1 = await requestLocal("GET", "/health");
  const r2 = await requestLocal("POST", "/api/echo-upper", JSON.stringify({ message: "hello slice stream" }));
  const r3 = await requestLocal("GET", "/missing");
  server.close();
  console.log(
    JSON.stringify({
      framework: "03-node-http-express-compat",
      totalRequestsDispatched: 3,
      health: JSON.parse(r1.text),
      poweredBy: r1.headers["x-powered-by"],
      echoStatus: r2.status,
      echoUpper: r2.text,
      notFoundStatus: r3.status
    })
  );
})();
