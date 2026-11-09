import crypto from "node:crypto";
import sliceEntry from "./build/slice-entry.js";

interface SvelteKitBenchmarkSummary {
    framework: string;
    libraries: string[];
    totalRequestsDispatched: number;
    status200Count: number;
    status201Count: number;
    status404Count: number;
    ssrPagesRendered: number;
    dynamicSlugPagesRendered: number;
    formActionsExecuted: number;
    clientDataLoads: number;
    cookiesIssued: number;
    hooksHeadersVerified: boolean;
    auditDigest: string;
}

const TENANTS = ["acme", "globex", "initech", "umbrella"];
const THEMES = ["synthwave", "cyberpunk", "dracula", "nord"];

async function runSvelteKitSuite(): Promise<SvelteKitBenchmarkSummary> {
    let totalRequestsDispatched = 0;
    let status200Count = 0;
    let status201Count = 0;
    let status404Count = 0;
    let ssrPagesRendered = 0;
    let dynamicSlugPagesRendered = 0;
    let formActionsExecuted = 0;
    let clientDataLoads = 0;
    let cookiesIssued = 0;
    let hooksHeadersVerified = true;
    const digestParts: string[] = [];

    // 1. Dispatch 4 SSR GET `/?name=<tenant>` requests with `Cookie: theme=<theme>`
    for (let i = 0; i < 4; i++) {
        const tenant = TENANTS[i];
        const theme = THEMES[i];
        const req = new Request(`https://edge.slice.local/?name=${tenant}`, {
            method: "GET",
            headers: {
                Cookie: `theme=${theme}`,
                "X-Forwarded-For": `10.0.0.${i + 1}`
            }
        });
        const res = await sliceEntry.fetch(req, {}, {});
        totalRequestsDispatched++;
        if (res.status === 200) status200Count++;
        if (res.headers.get("x-powered-by") !== "SvelteKit-on-Slice") {
            hooksHeadersVerified = false;
        }
        const html = await res.text();
        if (
            html.includes(`data-theme="${theme}"`) &&
            html.includes(`Hello, <strong>${tenant}</strong>!`) &&
            html.includes(`<span id="hits">${i + 1}</span>`)
        ) {
            ssrPagesRendered++;
        }
        digestParts.push(`ssr:${res.status}:${tenant}:${theme}:${i + 1}`);
    }

    // 2. Dispatch 2 Dynamic Route `[slug]` SSR GET `/items/:slug` requests
    for (let i = 0; i < 2; i++) {
        const slug = `quantum-edge-node-${i + 1}`;
        const req = new Request(`https://edge.slice.local/items/${slug}`, {
            method: "GET"
        });
        const res = await sliceEntry.fetch(req, {}, {});
        totalRequestsDispatched++;
        if (res.status === 200) status200Count++;
        if (res.headers.get("x-powered-by") !== "SvelteKit-on-Slice") {
            hooksHeadersVerified = false;
        }
        const html = await res.text();
        if (
            html.includes(`<h2>Item Detail: ${slug}</h2>`) &&
            html.includes(`Request ID: <code>req-slice-edge</code>`)
        ) {
            dynamicSlugPagesRendered++;
        }
        digestParts.push(`slug:${res.status}:${slug}`);
    }

    // 3. Dispatch 2 Form Action POST `/?/setTheme` requests
    for (let i = 0; i < 2; i++) {
        const nextTheme = THEMES[(i + 1) % THEMES.length];
        const req = new Request("https://edge.slice.local/?/setTheme", {
            method: "POST",
            headers: {
                Origin: "https://edge.slice.local",
                "Content-Type": "application/x-www-form-urlencoded",
                Accept: "application/json"
            },
            body: `theme=${nextTheme}`
        });
        const res = await sliceEntry.fetch(req, {}, {});
        totalRequestsDispatched++;
        if (res.status === 200) status200Count++;
        const setCookie = res.headers.get("set-cookie") || "";
        if (setCookie.includes(`theme=${nextTheme}`)) {
            cookiesIssued++;
        }
        const actionJson = (await res.json()) as { type: string; status: number; data: string };
        if (actionJson.type === "success" && actionJson.data.includes(nextTheme)) {
            formActionsExecuted++;
        }
        digestParts.push(`action:${res.status}:${actionJson.type}:${nextTheme}`);
    }

    // 4. Dispatch 1 Client-Side Data Navigation GET `/__data.json?name=Navigator`
    {
        const req = new Request("https://edge.slice.local/__data.json?name=Navigator", {
            method: "GET"
        });
        const res = await sliceEntry.fetch(req, {}, {});
        totalRequestsDispatched++;
        if (res.status === 200) status200Count++;
        const dataText = await res.text();
        if (dataText.includes(`"Navigator"`) && dataText.includes(`"WinterTC-Edge-Isolate"`)) {
            clientDataLoads++;
        }
        digestParts.push(`data:${res.status}:${clientDataLoads}`);
    }

    // 5. Dispatch 1 GET `/api/items?tag=edge` and 1 POST `/api/items`
    {
        const getReq = new Request("https://edge.slice.local/api/items?tag=edge", {
            method: "GET"
        });
        const getRes = await sliceEntry.fetch(getReq, {}, {});
        totalRequestsDispatched++;
        if (getRes.status === 200) status200Count++;
        const getBody = (await getRes.json()) as { ok: boolean; tag: string; count: number };
        digestParts.push(`api-get:${getRes.status}:${getBody.tag}:${getBody.count}`);

        const postReq = new Request("https://edge.slice.local/api/items", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ item: "sveltekit-5-cas" })
        });
        const postRes = await sliceEntry.fetch(postReq, {}, {});
        totalRequestsDispatched++;
        if (postRes.status === 201) status201Count++;
        const postBody = (await postRes.json()) as { created: boolean; count: number };
        digestParts.push(`api-post:${postRes.status}:${postBody.created}:${postBody.count}`);
    }

    // 6. Dispatch 1 unknown route probe (expect 404 `+error.svelte` SSR fallback)
    {
        const notFoundReq = new Request("https://edge.slice.local/non-existent-sveltekit-route", {
            method: "GET"
        });
        const notFoundRes = await sliceEntry.fetch(notFoundReq, {}, {});
        totalRequestsDispatched++;
        if (notFoundRes.status === 404) status404Count++;
        const errHtml = await notFoundRes.text();
        const has404Markup = errHtml.includes("<h1>404</h1>");
        digestParts.push(`404:${notFoundRes.status}:${has404Markup}`);
    }

    const auditDigest = crypto
        .createHash("sha256")
        .update(digestParts.join("|"))
        .digest("hex")
        .slice(0, 16);

    return {
        framework: "04-sveltekit-edge-ssr",
        libraries: [
            "@sveltejs/kit@3.1.1",
            "clsx@2.1.1",
            "cookie@2.0.2",
            "devalue@6.0.3",
            "set-cookie-parser@3.2.2",
            "svelte@5.57.3"
        ],
        totalRequestsDispatched,
        status200Count,
        status201Count,
        status404Count,
        ssrPagesRendered,
        dynamicSlugPagesRendered,
        formActionsExecuted,
        clientDataLoads,
        cookiesIssued,
        hooksHeadersVerified,
        auditDigest
    };
}

runSvelteKitSuite().then((summary) => {
    console.log(JSON.stringify(summary));
});

