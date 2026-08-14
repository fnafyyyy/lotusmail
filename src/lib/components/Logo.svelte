<script lang="ts">
  // Znak lotusMail: kwiat en face, trzy plany płatków.
  // Płatki tła i środka rozdzielone kolorem (chłodne płatki, ciepły środek).
  let { size = 24, mono = false }: { size?: number; mono?: boolean } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 64 64" fill="none" aria-label="lotusMail">
  <g fill="currentColor" class="text-accent">
    <path d="M32 47C21.4 49 13.3 41 13.8 34.7c5.6-2.8 16.1 1.7 18.2 12.3Z" opacity=".48" />
    <path d="M32 47c2.1-10.6 12.6-15.1 18.2-12.3C50.7 41 42.6 49 32 47Z" opacity=".48" />
    <path d="M32 47c-11.9-3.8-16.3-16.6-12.6-22.7C26.6 24.4 35.1 34.9 32 47Z" opacity=".72" />
    <path d="M32 47c-3.1-12.1 5.4-22.6 12.6-22.7C48.3 30.4 43.9 43.2 32 47Z" opacity=".72" />
  </g>
  <path
    d="M32 47c-9.2-10.3-6.6-25.4 0-29.5 6.6 4.1 9.2 19.2 0 29.5Z"
    fill="currentColor"
    class={mono ? "text-accent" : "text-flag"}
  />
</svg>
