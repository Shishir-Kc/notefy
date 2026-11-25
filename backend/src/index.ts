//! Notefy cloud backend: a Worker that stores vault files in Backblaze B2, keyed by their path inside the vault,
//! and serves share links (see share.ts).
//!
//! App routes need `Authorization: Bearer <API_TOKEN>`:
//!   PUT    /files/<vault>/<path>   upload (Content-Length required; `x-mtime` = local file time;
//!                                  409 if the file has web edits the app hasn't seen, unless `x-force: 1`)
//!   GET    /files/<vault>/<path>   download (`x-amz-meta-src: web` = edited in the browser)
//!   DELETE /files/<vault>/<path>   delete
//!   GET    /files/<vault>/<dir>/   JSON array of every key under that folder (recursive)
//!   POST /shares, GET /shares?vault=, DELETE /shares/<id>   manage share links
//! Public: GET/POST /s/<id>/...     the share viewer
import { bucket, sameSecret, toKey } from "./b2.ts";
import { shares, viewer } from "./share.ts";

/** Logs a B2 error and passes its status (and XML error body) back to the client. */
function fail(r: Response, op: string, key: string): Response {
	console.error(JSON.stringify({ op, key, status: r.status }));
	return new Response(r.body, { status: r.status });
}

async function files(request: Request, env: Env, pathname: string): Promise<Response> {
	const target = toKey(pathname);
	if (!target) return new Response("bad path", { status: 400 });
	const b2 = bucket(env);
	if (target.list) {
		if (request.method !== "GET") return new Response("method not allowed", { status: 405 });
		return Response.json(await b2.list(target.key));
	}
	const { key } = target;
	switch (request.method) {
		case "GET": {
			const r = await b2.get(key);
			return r.ok ? new Response(r.body, r) : fail(r, "get", key);
		}
		case "DELETE": {
			const r = await b2.delete(key);
			return r.ok ? new Response(null, { status: 204 }) : fail(r, "delete", key);
		}
		case "PUT": {
			const len = Number(request.headers.get("content-length"));
			if (!Number.isSafeInteger(len) || len < 0) return new Response("Content-Length required", { status: 411 });
			if (len > 0 && !request.body) return new Response("missing body", { status: 400 });
			if (request.headers.get("x-force") !== "1") {
				// don't let an app upload silently replace edits made through an editable share link
				const head = await b2.get(key, "HEAD");
				if (head.headers.get("x-amz-meta-src") === "web") return new Response("file has web edits", { status: 409 });
			}
			// B2 needs a known length, so stream through a FixedLengthStream instead of buffering the file.
			const body = len === 0 ? "" : request.body!.pipeThrough(new FixedLengthStream(len));
			const type = request.headers.get("content-type") ?? "application/octet-stream";
			const r = await b2.put(key, body, type, { src: "app", mtime: request.headers.get("x-mtime") ?? "" });
			return r.ok ? new Response(null, { status: 204 }) : fail(r, "put", key);
		}
	}
	return new Response("method not allowed", { status: 405 });
}

export default {
	async fetch(request, env): Promise<Response> {
		const url = new URL(request.url);
		try {
			if (url.pathname.startsWith("/s/")) return await viewer(request, env, url);
			const given = request.headers.get("authorization")?.replace(/^Bearer /, "") ?? "";
			if (!env.API_TOKEN || !(await sameSecret(given, env.API_TOKEN))) return new Response("unauthorized", { status: 401 });
			if (url.pathname === "/shares" || url.pathname.startsWith("/shares/")) return await shares(request, env, url);
			return await files(request, env, url.pathname);
		} catch (e) {
			console.error(JSON.stringify({ path: url.pathname, error: String(e) }));
			return new Response("storage error", { status: 502 });
		}
	},
} satisfies ExportedHandler<Env>;
