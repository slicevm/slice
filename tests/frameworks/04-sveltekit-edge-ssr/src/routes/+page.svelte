<script>
    let { data, form } = $props();
</script>

<main data-theme={data.theme}>
    <h1>{data.title}</h1>
    <p class="greeting">Hello, <strong>{data.visitor}</strong>!</p>
    <p class="stats">Runtime: <code>{data.runtime}</code> | Request ID: <code>{data.requestId}</code> | Theme: <code>{data.theme}</code> | KV Page Views: <span id="hits">{data.hits}</span></p>
    {#if form?.success}
        <p class="action-result">Updated theme to: <strong>{form.theme}</strong></p>
    {/if}
</main>
