// Local stand-in for Backblaze B2's S3 API, for dev only: no auth or signature checks.
// Objects are plain files under <dir>/<bucket>/<key>, so the bucket's folder layout is visible on disk;
// content type and x-amz-meta-* headers live in <dir>/meta.json.
// Usage: node dev/mock-b2.mjs <dir> [port]
import fs from "node:fs";
import http from "node:http";
import path from "node:path";

const dir = path.resolve(process.argv[2] ?? "dev/.local-b2");
const port = Number(process.argv[3] ?? 9000);
const metaFile = path.join(dir, "meta.json");
fs.mkdirSync(dir, { recursive: true });
const meta = fs.existsSync(metaFile) ? JSON.parse(fs.readFileSync(metaFile, "utf8")) : {};
const saveMeta = () => fs.writeFileSync(metaFile, JSON.stringify(meta, null, 1));
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

http
	.createServer(async (req, res) => {
		const u = new URL(req.url, "http://x");
		const [bucket, ...rest] = u.pathname.slice(1).split("/").map(decodeURIComponent);
		const key = rest.join("/");
		const file = path.join(dir, bucket, key);
		if (!file.startsWith(path.join(dir, bucket) + path.sep) && key) return res.writeHead(400).end();
		const id = `${bucket}/${key}`;

		if (req.method === "GET" && u.searchParams.get("list-type") === "2") {
			const prefix = `${bucket}/${u.searchParams.get("prefix") ?? ""}`;
			const keys = Object.keys(meta).filter((k) => k.startsWith(prefix)).sort();
			const body = keys.map((k) => `<Contents><Key>${esc(k.slice(bucket.length + 1))}</Key></Contents>`).join("");
			return res.end(`<ListBucketResult><IsTruncated>false</IsTruncated>${body}</ListBucketResult>`);
		}
		if (req.method === "PUT") {
			const chunks = [];
			for await (const c of req) chunks.push(c);
			fs.mkdirSync(path.dirname(file), { recursive: true });
			fs.writeFileSync(file, Buffer.concat(chunks));
			const m = Object.fromEntries(Object.entries(req.headers).filter(([h]) => h.startsWith("x-amz-meta-")));
			meta[id] = { "content-type": req.headers["content-type"] ?? "application/octet-stream", ...m };
			saveMeta();
			console.log(`PUT    ${id}  (${m["x-amz-meta-src"] ?? "?"})`);
			return res.end();
		}
		if (req.method === "DELETE") {
			fs.rmSync(file, { force: true });
			delete meta[id];
			saveMeta();
			console.log(`DELETE ${id}`);
			return res.writeHead(204).end();
		}
		if (!meta[id] || !fs.existsSync(file)) return res.writeHead(404).end("<Error><Code>NoSuchKey</Code></Error>");
		res.writeHead(200, meta[id]);
		res.end(req.method === "HEAD" ? undefined : fs.readFileSync(file));
	})
	.listen(port, "127.0.0.1", () => console.log(`mock B2 on http://127.0.0.1:${port}, bucket files in ${dir}`));
