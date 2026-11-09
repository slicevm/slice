import fs from 'node:fs';
import path from 'node:path';

/**
 * @sveltejs/kit Adapter for Slice Runtime (`@slicevm/adapter-sveltekit`)
 * Supports SvelteKit 3 (`generateServerInstance`) and SvelteKit 2 (`generateManifest`).
 * Emits:
 *   - `build/client/` (static assets & immutable client chunks for `--static`)
 *   - `build/server/` (SvelteKit SSR server chunks + server instance)
 *   - `build/slice-entry.js` (WinterTC `export default { fetch }` entrypoint)
 */
export default function adapterSlice(options = {}) {
    const out = options.out || 'build';
    return {
        name: '@slicevm/adapter-sveltekit',
        async adapt(builder) {
            const tmp = builder.getBuildDirectory('adapter-slice');
            builder.rimraf(out);
            builder.rimraf(tmp);
            builder.mkdirp(tmp);
            builder.mkdirp(`${out}/client`);
            builder.mkdirp(`${out}/server`);

            // 1. Write client assets and prerendered pages to build/client
            builder.writeClient(`${out}/client`);
            builder.writePrerendered(`${out}/client`);

            // 2. Copy server bundle and generate server instance (SvelteKit 3) or manifest (SvelteKit 2)
            builder.copy(builder.getServerDirectory(), `${out}/server`);

            let entryHeader;
            if (typeof builder.generateServerInstance === 'function') {
                builder.generateServerInstance(`${out}/server/instance.js`, {
                    serverDirectory: `${out}/server`
                });
                entryHeader = `import { server } from './server/instance.js';\n`;
            } else {
                const manifestStr = builder.generateManifest({ relativePath: '.' });
                fs.writeFileSync(
                    `${out}/server/manifest.js`,
                    `export const manifest = ${manifestStr};\n`
                );
                entryHeader = `import { Server } from './server/index.js';\nimport { manifest } from './server/manifest.js';\nconst server = new Server(manifest);\n`;
            }

            // 3. Emit Slice WinterTC entrypoint (build/slice-entry.js)
            const entryCode = `${entryHeader}
let initPromise = null;

export default {
    async fetch(request, env, ctx) {
        if (!initPromise) {
            initPromise = server.init({ env: env || (typeof process !== 'undefined' && process.env) || {} });
        }
        await initPromise;
        return await server.respond(request, {
            getClientAddress() {
                return request.headers.get('x-forwarded-for') || '127.0.0.1';
            },
            platform: {
                env,
                ctx,
                Slice: typeof Slice !== 'undefined' ? Slice : undefined
            }
        });
    }
};
`;
            fs.writeFileSync(path.join(out, 'slice-entry.js'), entryCode);
            builder.log.minor(`Wrote Slice WinterTC entrypoint to ${out}/slice-entry.js and client assets to ${out}/client`);
        }
    };
}
