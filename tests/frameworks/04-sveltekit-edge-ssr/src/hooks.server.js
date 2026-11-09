/** @type {import('@sveltejs/kit').Handle} */
export async function handle({ event, resolve }) {
    event.locals.requestId = 'req-slice-edge';
    const response = await resolve(event);
    response.headers.set('x-powered-by', 'SvelteKit-on-Slice');
    return response;
}

/** @type {import('@sveltejs/kit').HandleServerError} */
export function handleError({ status, error }) {
    const code = status ?? error?.status;
    if (code !== 404) {
        console.error(error);
    }
}
