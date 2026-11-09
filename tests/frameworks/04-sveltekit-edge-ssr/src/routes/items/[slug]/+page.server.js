/** @type {import('./$types').PageServerLoad} */
export async function load({ params, locals }) {
    return {
        slug: params.slug,
        requestId: locals.requestId || 'none'
    };
}
