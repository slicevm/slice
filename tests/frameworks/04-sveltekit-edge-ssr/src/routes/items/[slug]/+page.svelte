<script>
    let { data } = $props();
</script>

<article>
    <h2>Item Detail: {data.slug}</h2>
    <p>Request ID: <code>{data.requestId}</code></p>
</article>
