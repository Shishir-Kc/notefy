//! Backblaze B2 access (S3-compatible API) and the path rules for bucket keys.
import { AwsClient } from "aws4fetch";

/** Checks a `/`-separated relative path: no empty, `.`, `..` or backslash segments. */
export function cleanPath(rel: string): string | null {
	const parts = rel.split("/");
	return parts.some((p) => p === "" || p === "." || p === ".." || p.includes("\\")) ? null : rel;
}

/** Turns a `/files/...` request path into a bucket key. A trailing `/` means "list". Null for unsafe or empty paths. */
export function toKey(pathname: string): { key: string; list: boolean } | null {
	if (!pathname.startsWith("/files/")) return null;
	let rest: string;
	try {
		rest = decodeURIComponent(pathname.slice("/files/".length));
	} catch {
		return null;
	}
	const list = rest.endsWith("/");
	const key = cleanPath(list ? rest.slice(0, -1) : rest);
	return key === null ? null : { key: key + (list ? "/" : ""), list };
}

/** Percent-encodes each segment of a key for use in a URL path. */
export const encodePath = (key: string) => key.split("/").map(encodeURIComponent).join("/");

/** B2 client for one request. Object metadata: `src` = who wrote it last (`app` | `web`), `mtime` = app file time at last app upload. */
export function bucket(env: Env) {
	const client = new AwsClient({
		accessKeyId: env.B2_KEY_ID,
		secretAccessKey: env.B2_APP_KEY,
		service: "s3",
		region: new URL(env.B2_ENDPOINT).hostname.split(".")[1], // s3.<region>.backblazeb2.com
	});
	const base = `${env.B2_ENDPOINT}/${env.B2_BUCKET}`;
	const url = (key: string) => `${base}/${encodePath(key)}`;
	return {
		get: (key: string, method = "GET") => client.fetch(url(key), { method }),
		delete: (key: string) => client.fetch(url(key), { method: "DELETE" }),
		put: (key: string, body: BodyInit, type: string, meta: { src: string; mtime: string }) =>
			client.fetch(url(key), {
				method: "PUT",
				body,
				headers: { "content-type": type, "x-amz-meta-src": meta.src, "x-amz-meta-mtime": meta.mtime },
			}),
		/** Every key under `prefix`, following ListObjectsV2 pagination. Throws if B2 fails. */
		async list(prefix: string): Promise<string[]> {
			const keys: string[] = [];
			let token = "";
			do {
				const q = new URLSearchParams({ "list-type": "2", prefix });
				if (token) q.set("continuation-token", token);
				const r = await client.fetch(`${base}?${q}`);
				if (!r.ok) throw new Error(`B2 list ${prefix} failed: ${r.status}`);
				const xml = await r.text(); // bounded: one page is at most 1000 keys
				for (const m of xml.matchAll(/<Key>(.*?)<\/Key>/g)) keys.push(unescapeXml(m[1]));
				token = xml.includes("<IsTruncated>true</IsTruncated>")
					? unescapeXml(xml.match(/<NextContinuationToken>(.*?)<\/NextContinuationToken>/)?.[1] ?? "")
					: "";
			} while (token);
			return keys;
		},
	};
}

const unescapeXml = (s: string) =>
	s.replace(/&(lt|gt|quot|apos|amp);/g, (_, e) => ({ lt: "<", gt: ">", quot: '"', apos: "'", amp: "&" })[e as string]!);

/** Constant-time string comparison (hashed first so length doesn't leak). */
export async function sameSecret(a: string, b: string): Promise<boolean> {
	const enc = new TextEncoder();
	const [x, y] = await Promise.all([a, b].map((s) => crypto.subtle.digest("SHA-256", enc.encode(s))));
	return crypto.subtle.timingSafeEqual(x, y);
}
