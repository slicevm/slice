/** @type {import('./$types').PageServerLoad} */
export async function load({ url, cookies, locals }) {
    const visitor = url.searchParams.get('name') || 'Edge Developer';
    const theme = cookies.get('theme') || 'light';
    const hits = typeof Slice !== 'undefined' && Slice.kv
        ? Slice.kv.incr('sveltekit_page_views', 1)
        : (globalThis.__sk_kv_views = (globalThis.__sk_kv_views || 0) + 1);

    return {
        title: 'SvelteKit 2 + Svelte 5 SSR on Slice',
        visitor,
        theme,
        requestId: locals.requestId || 'none',
        hits,
        runtime: 'WinterTC-Edge-Isolate'
    };
}

/** @type {import('./$types').Actions} */
export const actions = {
    setTheme: async ({ request, cookies }) => {
        const form = await request.formData();
        const theme = String(form.get('theme') || 'dark');
        cookies.set('theme', theme, { path: '/', httpOnly: true });
        return { success: true, theme };
    }
};
