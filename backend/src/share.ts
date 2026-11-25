//! Share links: records in KV (`SHARES`), the app's API to manage them, and the public viewer
//! (read-only, or editable when the link allows it). Pages are rendered here and run no scripts.
import { Marked } from "marked";
import { bucket, cleanPath, encodePath, sameSecret } from "./b2.ts";

/** One share link. `path` "" = the whole vault; `code` "" = anyone with the link can open it;
 *  `expires` = unix seconds when the link stops working, 0 = never. */
type Share = { vault: string; path: string; edit: boolean; code: string; expires: number };

/** True once a link's expiry time has passed (KV deletes it too, but only eventually). */
const expired = (s: Share) => !!s.expires && s.expires * 1000 <= Date.now();

const kvKey = (id: string) => `share:${id}`;
const ID_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"; // 64: no modulo bias
const CODE_CHARS = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // 32, no look-alikes (0/O, 1/I)

function random(len: number, chars: string): string {
	return Array.from(crypto.getRandomValues(new Uint8Array(len)), (b) => chars[b % chars.length]).join("");
}

/** App API (already authorized): create, list and delete share links. */
export async function shares(request: Request, env: Env, url: URL): Promise<Response> {
	if (url.pathname === "/shares" && request.method === "POST") {
		const b = await request.json<Record<string, unknown>>().catch(() => null);
		const vault = typeof b?.vault === "string" && !b.vault.includes("/") ? cleanPath(b.vault) : null;
		const path = typeof b?.path === "string" ? b.path : null;
		// seconds until the link expires; 0 or missing = never. KV's minimum TTL is 60s.
		const ttl = b?.expires_in ?? 0;
		if (!b || !vault || path === null || (path !== "" && !cleanPath(path))) return new Response("bad share", { status: 400 });
		if (typeof ttl !== "number" || !Number.isSafeInteger(ttl) || (ttl !== 0 && ttl < 60))
			return new Response("expires_in must be 0 or at least 60 seconds", { status: 400 });
		const share: Share = {
			vault,
			path,
			edit: b.edit === true,
			code: b.code === true ? random(8, CODE_CHARS) : "",
			expires: ttl ? Math.floor(Date.now() / 1000) + ttl : 0,
		};
		// the record lives in KV metadata (returned by list), which is capped at 1024 bytes
		if (new TextEncoder().encode(JSON.stringify(share)).length > 1000) return new Response("path too long", { status: 400 });
		const id = random(22, ID_CHARS);
		await env.SHARES.put(kvKey(id), "", ttl ? { metadata: share, expirationTtl: ttl } : { metadata: share });
		return Response.json({ id, ...share }, { status: 201 });
	}
	if (url.pathname === "/shares" && request.method === "GET") {
		const vault = url.searchParams.get("vault");
		const out = [];
		let cursor: string | undefined;
		do {
			const page = await env.SHARES.list<Share>({ prefix: "share:", cursor });
			for (const k of page.keys)
				if (k.metadata && !expired(k.metadata) && (!vault || k.metadata.vault === vault)) out.push({ id: k.name.slice("share:".length), ...k.metadata });
			cursor = page.list_complete ? undefined : page.cursor;
		} while (cursor);
		return Response.json(out);
	}
	const id = url.pathname.slice("/shares/".length);
	if (id && request.method === "DELETE") {
		await env.SHARES.delete(kvKey(id));
		return new Response(null, { status: 204 });
	}
	return new Response("not found", { status: 404 });
}

/** Public pages under `/s/<id>/<path inside the vault>`. */
export async function viewer(request: Request, env: Env, url: URL): Promise<Response> {
	const [id, ...rest] = url.pathname.slice("/s/".length).split("/");
	let rel: string;
	try {
		rel = decodeURIComponent(rest.join("/"));
	} catch {
		return notFound();
	}
	const share = id ? (await env.SHARES.getWithMetadata<Share>(kvKey(id))).metadata : null;
	if (!share) return page(404, "Link not found", "", "", `<p class="muted">This share link doesn't exist or was turned off.</p>`);
	if (expired(share)) return page(410, "Link expired", "", "", `<p class="muted">This share link has expired.</p>`);
	if (Number(request.headers.get("content-length")) > 5_000_000) return new Response("too large", { status: 413 });
	const form = request.method === "POST" ? await request.formData() : null;

	if (share.code) {
		const cookie = `nfy_${id}`;
		const proof = await sha256(`${id}:${share.code}`);
		const given = form?.get("code");
		if (typeof given === "string") {
			if (!(await sameSecret(given.trim().toUpperCase(), share.code))) return unlock(true);
			const attrs = `Path=/s/${id}/; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000`;
			return new Response(null, { status: 303, headers: { location: url.pathname, "set-cookie": `${cookie}=${proof}; ${attrs}` } });
		}
		const have = request.headers.get("cookie")?.match(new RegExp(`(?:^|; )${cookie}=([0-9a-f]+)`))?.[1] ?? "";
		if (!(await sameSecret(have, proof))) return unlock(false);
	}

	// a file share only ever exposes that file; hidden files (.notefy/...) are never served
	if (rel !== "" && (!cleanPath(rel) || rel.split("/").some((s) => s.startsWith(".")))) return notFound();
	if (share.path && rel !== "" && rel !== share.path) return notFound();
	const file = share.path || rel; // "" = the vault's start page
	const key = `${share.vault}/${file}`;
	const b2 = bucket(env);

	if (file && !file.toLowerCase().endsWith(".md")) {
		// attachments (images) next to the notes; sandboxed so an SVG can't run script on this origin
		const r = await b2.get(key);
		if (!r.ok) return notFound();
		const ext = file.split(".").pop()!.toLowerCase();
		return new Response(r.body, {
			headers: {
				"content-type": MIME[ext] ?? r.headers.get("content-type") ?? "application/octet-stream",
				"content-security-policy": "sandbox; default-src 'none'; style-src 'unsafe-inline'",
				"x-content-type-options": "nosniff",
			},
		});
	}

	if (request.method === "POST") {
		const text = form?.get("text");
		if (!share.edit || !file || typeof text !== "string") return new Response("read-only link", { status: 403 });
		const head = await b2.get(key, "HEAD");
		if (!head.ok) return notFound(); // only existing notes can be edited
		// keep the app's mtime so the app can tell whether its own copy changed since (see app's cloud.rs)
		const r = await b2.put(key, text.replace(/\r\n/g, "\n"), "text/markdown; charset=utf-8", {
			src: "web",
			mtime: head.headers.get("x-amz-meta-mtime") ?? "",
		});
		if (!r.ok) throw new Error(`B2 put ${key} failed: ${r.status}`);
		return new Response(null, { status: 303, headers: { location: url.pathname } });
	}

	const notes = share.path
		? []
		: (await b2.list(`${share.vault}/`))
				.map((k) => k.slice(share.vault.length + 1))
				.filter((r) => r.toLowerCase().endsWith(".md") && !r.split("/").some((s) => s.startsWith(".")));
	const side = share.path ? "" : sidebar(share.vault, notes, id, file);
	if (!file) return page(200, share.vault, side, "", `<p class="muted">Choose a note on the left.</p>`);

	const r = await b2.get(key);
	if (r.status === 404) return notFound();
	if (!r.ok) throw new Error(`B2 get ${key} failed: ${r.status}`);
	const text = await r.text(); // a single note: small
	const editing = share.edit && url.searchParams.has("edit");
	const crumbs = file
		.replace(/\.md$/i, "")
		.split("/")
		.map((p, i, all) => `<span${i + 1 === all.length ? ' class="here"' : ""}>${esc(p)}</span>`)
		.join('<span class="sep">/</span>');
	const actions = !share.edit
		? `<span class="badge">${I.eye} Read only</span>`
		: editing
			? `<a class="btn" href="${esc(url.pathname)}">Cancel</a><button class="btn primary" form="editor">${I.save} Save</button>`
			: `<a class="btn" href="?edit">${I.edit} Edit</a>`;
	const body = editing
		? `<form id="editor" method="post"><textarea name="text" spellcheck="false" autofocus>${esc(text)}</textarea></form>`
		: md.parse(wikilinks(text, notes, id), { async: false });
	return page(200, stem(file), side, `<div class="crumbs">${crumbs}</div><div class="actions">${actions}</div>`, `<h1 class="title">${esc(stem(file))}</h1>${body}`);
}

const MIME: Record<string, string> = {
	png: "image/png",
	jpg: "image/jpeg",
	jpeg: "image/jpeg",
	gif: "image/gif",
	webp: "image/webp",
	svg: "image/svg+xml",
	bmp: "image/bmp",
	pdf: "application/pdf",
};

/** Codicons from the app's bundled Nerd Font. */
const I = { file: "", folder: "", eye: "", edit: "", save: "", lock: "", vault: "" };

// raw HTML in notes is shown as text, never injected (shared pages may be editable by anyone with the link)
const md = new Marked({ gfm: true, breaks: true });
md.use({ renderer: { html: ({ text }) => esc(text) } });

export const esc = (s: string) =>
	s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const stem = (p: string) => p.split("/").pop()!.replace(/\.md$/i, "");

async function sha256(s: string): Promise<string> {
	const h = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s));
	return Array.from(new Uint8Array(h), (b) => b.toString(16).padStart(2, "0")).join("");
}

/** `[[target]]` / `[[target|alias]]` → Markdown links, resolved like the app does: vault path first, then file name. */
export function wikilinks(text: string, notes: string[], id: string): string {
	return text.replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_, target: string, alias?: string) => {
		const t = target.trim().replace(/\.md$/i, "").toLowerCase();
		const hit =
			notes.find((n) => n.slice(0, -3).toLowerCase() === t) ?? notes.find((n) => stem(n).toLowerCase() === t);
		const label = (alias ?? target).replace(/[[\]]/g, "\\$&");
		return hit ? `[${label}](</s/${id}/${encodePath(hit)}>)` : label;
	});
}

/** Folder tree like the app's sidebar: folders first, then notes, A→Z. Folders on the open note's path start open. */
function sidebar(vault: string, notes: string[], id: string, open: string): string {
	type Node = { dirs: Map<string, Node>; files: string[] };
	const root: Node = { dirs: new Map(), files: [] };
	for (const n of notes) {
		const parts = n.split("/");
		let node = root;
		for (const d of parts.slice(0, -1)) {
			if (!node.dirs.has(d)) node.dirs.set(d, { dirs: new Map(), files: [] });
			node = node.dirs.get(d)!;
		}
		node.files.push(n);
	}
	const by = (a: string, b: string) => a.toLowerCase().localeCompare(b.toLowerCase());
	const render = (node: Node, prefix: string): string =>
		[...node.dirs.keys()]
			.sort(by)
			.map((d) => {
				const path = prefix + d + "/";
				return `<details${open.startsWith(path) ? " open" : ""}><summary>${I.folder} ${esc(d)}</summary>${render(node.dirs.get(d)!, path)}</details>`;
			})
			.join("") +
		node.files
			.sort(by)
			.map((f) => `<a class="file${f === open ? " here" : ""}" href="/s/${id}/${esc(encodePath(f))}">${I.file} ${esc(stem(f))}</a>`)
			.join("");
	return `<aside><div class="vault">${I.vault} ${esc(vault)}</div><nav class="tree">${render(root, "")}</nav></aside>`;
}

function unlock(wrong: boolean): Response {
	return page(
		401,
		"Access code",
		"",
		"",
		`<form class="card" method="post">
			<div class="card-title">${I.lock} Access code</div>
			<p class="muted">This shared note is protected. Enter the code you were given.</p>
			<input name="code" autocomplete="off" autocapitalize="characters" autofocus placeholder="XXXXXXXX">
			${wrong ? `<p class="error">That code doesn't match.</p>` : ""}
			<button class="btn primary">Open</button>
		</form>`,
	);
}

const notFound = () => page(404, "Not found", "", "", `<p class="muted">Nothing is shared at this address.</p>`);

/** Full HTML page in the app's dark theme. No scripts: the CSP forbids them, so note content can't run code. */
function page(status: number, title: string, side: string, bar: string, content: string): Response {
	const nonce = random(16, ID_CHARS);
	const csp = `default-src 'none'; style-src 'nonce-${nonce}'; img-src 'self' https: data:; font-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'`;
	const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex"><title>${esc(title)} · Notefy</title>
<style nonce="${nonce}">${CSS}</style></head>
<body><div class="app">${side}<main>${bar ? `<header class="bar">${bar}</header>` : ""}<article>${content}</article></main></div></body></html>`;
	return new Response(html, {
		status,
		headers: { "content-type": "text/html; charset=utf-8", "content-security-policy": csp, "referrer-policy": "no-referrer" },
	});
}

// Palette and type from the app's theme.rs (DARK) and its bundled JetBrains Mono Nerd Font.
const CSS = `
@font-face{font-family:"JBM";src:url(/JetBrainsMonoNerdFont-Regular.ttf)}
@font-face{font-family:"JBM";font-weight:bold;src:url(/JetBrainsMonoNerdFont-Bold.ttf)}
@font-face{font-family:"JBM";font-style:italic;src:url(/JetBrainsMonoNerdFont-Italic.ttf)}
:root{--bg:#1e1e1e;--panel:#262626;--surface:#363636;--text:#f3efe6;--muted:#9a968e;--accent:#e8e2d4;--border:#333;--link:#d9c49a;color-scheme:dark}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.6 "JBM",ui-monospace,monospace}
.app{display:flex;min-height:100vh}
aside{width:290px;flex:none;background:var(--panel);padding:12px 8px;overflow:auto;max-height:100vh;position:sticky;top:0}
.vault{color:var(--muted);font-size:13px;padding:4px 8px 10px}
.tree a,.tree summary{display:block;padding:3px 8px;border-radius:4px;color:var(--text);text-decoration:none;cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-size:14px}
.tree summary{list-style:none}.tree summary::-webkit-details-marker{display:none}
.tree details>details,.tree details>a{margin-left:14px}
.tree a:hover,.tree summary:hover{background:var(--surface)}
.tree a.here{background:var(--surface);color:var(--accent)}
main{flex:1;min-width:0}
.bar{display:flex;align-items:center;gap:12px;height:36px;padding:0 16px;font-size:13.5px}
.crumbs{flex:1;min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;color:var(--muted)}
.crumbs .here{color:var(--text)}.crumbs .sep{color:#5c5a55;margin:0 6px}
.actions{display:flex;gap:6px;flex:none}
.btn,.badge{font:inherit;font-size:13px;padding:4px 10px;border-radius:4px;border:0;background:transparent;color:var(--text);text-decoration:none;cursor:pointer}
.btn:hover{background:var(--surface);color:var(--accent)}
.btn.primary{background:var(--surface)}
.badge{color:var(--muted);cursor:default}
article{max-width:760px;margin:0 auto;padding:24px 32px 96px}
.title{font-size:30px;line-height:1.25;margin:8px 0 24px}
h1,h2,h3,h4{line-height:1.3;margin:1.4em 0 .5em}
a{color:var(--link)}
code{background:var(--surface);padding:1px 5px;border-radius:4px;font-size:14.5px}
pre{background:var(--panel);border:1px solid var(--border);border-radius:6px;padding:12px 14px;overflow:auto}
pre code{background:none;padding:0}
blockquote{margin:0;padding:2px 16px;border-left:3px solid var(--link);color:var(--muted)}
table{border-collapse:collapse;display:block;overflow:auto}
th,td{border:1px solid var(--border);padding:6px 12px}
th{background:var(--panel)}
img{max-width:100%}
hr{border:0;border-top:1px solid var(--border)}
ul:has(>li>input[type=checkbox]){list-style:none;padding-left:4px}
input[type=checkbox]{accent-color:var(--link)}
.muted{color:var(--muted)}
textarea{width:100%;min-height:65vh;background:transparent;color:var(--text);border:0;outline:0;resize:vertical;font:inherit;line-height:1.6}
.card{max-width:360px;margin:15vh auto 0;background:var(--panel);border:1px solid var(--border);border-radius:8px;padding:20px;display:flex;flex-direction:column;gap:10px}
.card-title{font-size:16px;font-weight:bold}
.card p{margin:0;font-size:13.5px}
.card input{font:inherit;font-size:18px;letter-spacing:3px;text-transform:uppercase;padding:8px 10px;background:var(--bg);color:var(--text);border:1px solid var(--border);border-radius:4px}
.card .btn{align-self:flex-end}
.error{color:#e0796f}
@media (max-width:700px){.app{flex-direction:column}aside{width:auto;max-height:40vh;position:static}article{padding:16px 16px 64px}}
`;
