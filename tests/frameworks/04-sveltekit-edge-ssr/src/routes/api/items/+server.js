import { json } from '@sveltejs/kit';

/** @type {import('./$types').RequestHandler} */
export async function GET({ url }) {
    const tag = url.searchParams.get('tag') || 'all';
    const count = typeof Slice !== 'undefined' && Slice.kv
        ? Slice.kv.incr('sveltekit_api_calls', 1)
        : (globalThis.__sk_kv_api = (globalThis.__sk_kv_api || 0) + 1);

    return json({
        ok: true,
        framework: 'SvelteKit',
        tag,
        count,
        items: ['slice-cas', 'slice-engine', 'slice-sandbox']
    });
}

/** @type {import('./$types').RequestHandler} */
export async function POST({ request }) {
    const body = await request.json();
    const count = typeof Slice !== 'undefined' && Slice.kv
        ? Slice.kv.incr('sveltekit_api_posts', 1)
        : (globalThis.__sk_kv_posts = (globalThis.__sk_kv_posts || 0) + 1);

    return json({
        created: true,
        count,
        received: body
    }, { status: 201 });
}
