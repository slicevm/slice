import { describe, test, expect } from "slice:test";
import sliceEntry from "../src/build/slice-entry.js";

describe("SvelteKit 3 + Svelte 5 Edge SSR & Actions Suite", () => {
    test("renders Svelte 5 SSR page with Cookie, hooks.server.js locals, and Slice.kv state", async () => {
        const res = await sliceEntry.fetch(
            new Request("https://edge.slice.local/?name=SliceTest", {
                headers: {
                    Cookie: "theme=synthwave"
                }
            }),
            {},
            {}
        );
        expect(res.status).toBe(200);
        expect(res.headers.get("x-powered-by")).toBe("SvelteKit-on-Slice");
        const html = await res.text();
        expect(html).toContain('data-theme="synthwave"');
        expect(html).toContain("Hello, <strong>SliceTest</strong>!");
        expect(html).toContain('<span id="hits">1</span>');
    });

    test("rolls back Slice.kv between tests and executes Form Action with Set-Cookie", async () => {
        // Verify Time-Travel KV rollback reset `sveltekit_page_views` back to 0 -> 1
        const ssrRes = await sliceEntry.fetch(
            new Request("https://edge.slice.local/?name=RollbackCheck"),
            {},
            {}
        );
        const ssrHtml = await ssrRes.text();
        expect(ssrHtml).toContain('<span id="hits">1</span>');

        // Execute SvelteKit Form Action `POST /?/setTheme`
        const actionRes = await sliceEntry.fetch(
            new Request("https://edge.slice.local/?/setTheme", {
                method: "POST",
                headers: {
                    Origin: "https://edge.slice.local",
                    "Content-Type": "application/x-www-form-urlencoded",
                    Accept: "application/json"
                },
                body: "theme=cyberpunk"
            }),
            {},
            {}
        );
        expect(actionRes.status).toBe(200);
        expect(actionRes.headers.get("set-cookie") || "").toContain("theme=cyberpunk");
        const body = await actionRes.json();
        expect(body.type).toBe("success");
    });
});
