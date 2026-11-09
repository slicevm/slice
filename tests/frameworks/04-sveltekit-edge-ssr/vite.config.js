import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import adapterSlice from './adapter-slice.js';

export default defineConfig({
    plugins: [
        sveltekit({
            adapter: adapterSlice({ out: 'src/build' })
        })
    ]
});
