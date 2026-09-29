// gaskey ui — static server for the browser session-key app
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const port = Number(process.argv[2] || 8787);

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png" };
createServer((req, res) => {
  const path = req.url === "/" ? "/index.html" : req.url.split("?")[0];
  try {
    const body = readFileSync(join(dir, path));
    res.writeHead(200, { "Content-Type": types[path.slice(-5).replace(/^\./, m => m)] || types[path.split(".").pop()] || "text/plain" });
    res.end(body);
  } catch {
    res.writeHead(404); res.end("not found");
  }
}).listen(port, () => {
  console.log(`gaskey ui → http://localhost:${port}`);
});
