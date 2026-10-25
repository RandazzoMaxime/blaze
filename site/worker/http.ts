/** Small helpers shared by the routes: cookies, origin checks, redirects, tokens. */

export function json(body: unknown, status = 200, headers: HeadersInit = {}): Response {
	const h = new Headers(headers);
	h.set('content-type', 'application/json; charset=utf-8');
	h.set('cache-control', 'no-store');
	return new Response(JSON.stringify(body), { status, headers: h });
}

export function plain(body: string, status: number, headers: HeadersInit = {}): Response {
	const h = new Headers(headers);
	h.set('content-type', 'text/plain; charset=utf-8');
	h.set('cache-control', 'no-store');
	return new Response(body, { status, headers: h });
}

export function methodNotAllowed(allow: string): Response {
	return plain('Method not allowed.', 405, { allow });
}

const NEXT_MAX = 1024;
const NEXT_BASE = 'https://x.invalid';

/**
 * A same-site path to send the browser back to, or "/". The value is resolved against a placeholder origin the way
 * a browser would resolve it, so "//host", "/\host", "/.//host" and the like (another host) fail, and what comes
 * back is the parsed path, query and fragment: percent-encoded ASCII, safe in a Location header. Input or output
 * longer than 1024 chars: "/".
 */
export function safeNext(v: string | null | undefined): string {
	if (typeof v !== 'string' || v.length > NEXT_MAX || !v.startsWith('/')) return '/';
	// URL parsers drop tab and newline, which could turn "/\t/evil" into "//evil"; refuse control characters outright.
	if (/[\u0000-\u001f\u007f]/.test(v)) return '/';
	let u: URL;
	try {
		u = new URL(v, NEXT_BASE);
	} catch {
		return '/';
	}
	if (u.origin !== NEXT_BASE) return '/';
	const out = u.pathname + u.search + u.hash;
	// "/.//host" parses to the path "//host", which a browser would read as another host.
	return out.length <= NEXT_MAX && /^\/[\x21-\x7e]*$/.test(out) && !out.startsWith('//') ? out : '/';
}

/** A 303. A location the Headers API refuses becomes "/", so building the response never throws. */
export function redirect(location: string, cookies: string[] = []): Response {
	const h = new Headers({ 'cache-control': 'no-store' });
	try {
		h.set('location', location);
	} catch {
		h.set('location', '/');
	}
	for (const c of cookies) h.append('set-cookie', c);
	return new Response(null, { status: 303, headers: h });
}

/** True when the request's Origin header is this site's origin. A missing Origin fails. */
export function sameOrigin(request: Request): boolean {
	return request.headers.get('origin') === new URL(request.url).origin;
}

export function cookie(request: Request, name: string): string | null {
	const header = request.headers.get('cookie');
	if (!header) return null;
	for (const part of header.split(';')) {
		const i = part.indexOf('=');
		if (i < 0) continue;
		if (part.slice(0, i).trim() === name) return part.slice(i + 1).trim();
	}
	return null;
}

/**
 * An HttpOnly, Secure, SameSite=Lax cookie on Path=/ with no Domain, as the `__Host-` prefix requires (browsers drop a
 * `__Host-` cookie set any other way). Browsers count http://localhost as secure, so `wrangler dev` still works.
 */
export function setCookie(name: string, value: string, maxAge: number): string {
	return `${name}=${value}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=${maxAge}`;
}

export const clearCookie = (name: string) => setCookie(name, '', 0);

/** 32 random bytes, base64url (43 chars, no padding). */
export function randomToken(): string {
	const bytes = crypto.getRandomValues(new Uint8Array(32));
	let s = '';
	for (const b of bytes) s += String.fromCharCode(b);
	return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export async function sha256hex(s: string): Promise<string> {
	const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s));
	return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** Constant-time string comparison (the length is not secret). */
export function timingSafeEqual(a: string, b: string): boolean {
	const x = new TextEncoder().encode(a);
	const y = new TextEncoder().encode(b);
	if (x.byteLength !== y.byteLength) return false;
	return crypto.subtle.timingSafeEqual(x, y);
}

/** The media type of a Content-Type header, lowercased, without parameters. */
export function mediaType(request: Request): string {
	return (request.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase();
}

/**
 * The request body, at most `max` bytes, as a Response (for .text() / .formData() with the request's Content-Type),
 * or null when it is longer. A Content-Length over `max` is refused before anything is read; without one (chunked),
 * reading stops as soon as the bytes pass `max`.
 */
export async function boundedBody(request: Request, max: number): Promise<Response | null> {
	const len = request.headers.get('content-length');
	if (len !== null && !(/^[0-9]{1,15}$/.test(len.trim()) && Number(len.trim()) <= max)) return null;
	const headers = { 'content-type': request.headers.get('content-type') ?? '' };
	if (!request.body) return new Response('', { headers });
	const reader = request.body.getReader();
	const chunks: Uint8Array[] = [];
	let size = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		size += value.byteLength;
		if (size > max) {
			await reader.cancel();
			return null;
		}
		chunks.push(value);
	}
	const bytes = new Uint8Array(size);
	let at = 0;
	for (const c of chunks) {
		bytes.set(c, at);
		at += c.byteLength;
	}
	return new Response(bytes, { headers });
}

const TURNSTILE = 'https://challenges.cloudflare.com';

/**
 * The Content-Security-Policy of the HTML the worker serves: scripts, styles and fetches from this origin only, avatars
 * from GitHub, narration audio (media-src) from this origin, forms posting back here. `turnstile` (only /updates) also lets in the Turnstile script and its frame.
 * static/_headers gives the static pages the same policy, less the avatars and Turnstile; change both together.
 */
export function csp({ turnstile = false } = {}): string {
	return [
		"default-src 'self'",
		turnstile ? `script-src 'self' ${TURNSTILE}` : "script-src 'self'",
		"style-src 'self'",
		"img-src 'self' https://avatars.githubusercontent.com https://github.com",
		"connect-src 'self'",
		"media-src 'self'",
		turnstile ? `frame-src ${TURNSTILE}` : "frame-src 'none'",
		"form-action 'self'",
		"frame-ancestors 'none'",
		"base-uri 'none'",
		"object-src 'none'",
	].join('; ');
}
