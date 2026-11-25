//! Notefy cloud backend: a Worker that stores vault files in a Backblaze B2 bucket, keyed by their path inside the vault.
//!
//! All routes need `Authorization: Bearer <API_TOKEN>`.
//!   PUT    /files/<vault>/<path>   upload (body = file bytes, Content-Length required)
//!   GET    /files/<vault>/<path>   download
//!   DELETE /files/<vault>/<path>   delete
//!   GET    /files/<vault>/<dir>/   JSON array of every key under that folder (recursive)
import { AwsClient } from "aws4fetch";

/** Turns a request path into a bucket key. A trailing `/` means "list". Null for unsafe or empty paths. */
export function toKey(pathname: string): { key: string; list: boolean } | null {
	if (!pathname.startsWith("/files/")) return null;
	let rest: string;
	try {
		rest = decodeURIComponent(pathname.slice("/files/".length));
	} catch {
		return null;
	}
	const list = rest.endsWith("/");
	const parts = (list ? rest.slice(0, -1) : rest).split("/");
	if (parts.some((p) => p === "" || p === "." || p === ".." || p.includes("\\"))) return null;
	return { key: parts.join("/") + (list ? "/" : ""), list };
}

/** Constant-time check of the bearer token (hashed first so length doesn't leak). */
async function authorized(request: Request, token: string): Promise<boolean> {
	const given = request.headers.get("authorization")?.replace(/^Bearer /, "") ?? "";
	if (!token) return false;
	const enc = new TextEncoder();
	const [a, b] = await Promise.all([given, token].map((s) => crypto.subtle.digest("SHA-256", enc.encode(s))));
	return crypto.subtle.timingSafeEqual(a, b);
}

const unescapeXml = (s: string) =>
	s.replace(/&(lt|gt|quot|apos|amp);/g, (_, e) => ({ lt: "<", gt: ">", quot: '"', apos: "'", amp: "&" })[e as string]!);

/** Every key under `prefix`, following ListObjectsV2 pagination. */
async function listKeys(b2: AwsClient, base: string, prefix: string): Promise<Response> {
	const keys: string[] = [];
	let token = "";
	do {
		const q = new URLSearchParams({ "list-type": "2", prefix });
		if (token) q.set("continuation-token", token);
		const r = await b2.fetch(`${base}?${q}`);
		if (!r.ok) return fail(r, "list", prefix);
		const xml = await r.text(); // bounded: one page is at most 1000 keys
		for (const m of xml.matchAll(/<Key>(.*?)<\/Key>/g)) keys.push(unescapeXml(m[1]));
		token = xml.includes("<IsTruncated>true</IsTruncated>")
			? unescapeXml(xml.match(/<NextContinuationToken>(.*?)<\/NextContinuationToken>/)?.[1] ?? "")
			: "";
	} while (token);
	return Response.json(keys);
}

/** Logs a B2 error and passes its status (and XML error body) back to the client. */
function fail(r: Response, op: string, key: string): Response {
	console.error(JSON.stringify({ op, key, status: r.status }));
	return new Response(r.body, { status: r.status });
}

export default {
	async fetch(request, env): Promise<Response> {
		if (!(await authorized(request, env.API_TOKEN))) return new Response("unauthorized", { status: 401 });
		const target = toKey(new URL(request.url).pathname);
		if (!target) return new Response("bad path", { status: 400 });

		const b2 = new AwsClient({
			accessKeyId: env.B2_KEY_ID,
			secretAccessKey: env.B2_APP_KEY,
			service: "s3",
			region: new URL(env.B2_ENDPOINT).hostname.split(".")[1], // s3.<region>.backblazeb2.com
		});
		const base = `${env.B2_ENDPOINT}/${env.B2_BUCKET}`;

		if (target.list) {
			if (request.method !== "GET") return new Response("method not allowed", { status: 405 });
			return listKeys(b2, base, target.key);
		}

		const url = `${base}/${target.key.split("/").map(encodeURIComponent).join("/")}`;
		switch (request.method) {
			case "GET": {
				const r = await b2.fetch(url);
				return r.ok ? new Response(r.body, r) : fail(r, "get", target.key);
			}
			case "DELETE": {
				const r = await b2.fetch(url, { method: "DELETE" });
				return r.ok ? new Response(null, { status: 204 }) : fail(r, "delete", target.key);
			}
			case "PUT": {
				const len = Number(request.headers.get("content-length"));
				if (!Number.isSafeInteger(len) || len < 0) return new Response("Content-Length required", { status: 411 });
				if (len > 0 && !request.body) return new Response("missing body", { status: 400 });
				// B2 needs a known length, so stream through a FixedLengthStream instead of buffering the file.
				const body = len === 0 ? "" : request.body!.pipeThrough(new FixedLengthStream(len));
				const r = await b2.fetch(url, {
					method: "PUT",
					body,
					headers: { "content-type": request.headers.get("content-type") ?? "application/octet-stream" },
				});
				return r.ok ? new Response(null, { status: 204 }) : fail(r, "put", target.key);
			}
		}
		return new Response("method not allowed", { status: 405 });
	},
} satisfies ExportedHandler<Env>;
