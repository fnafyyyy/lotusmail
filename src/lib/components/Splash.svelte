<script lang="ts">
  // Ekran powitalny: płatki lotosu rozwijają się od środka, wokół pulsuje
  // poświata. Znika, gdy skrzynka jest wczytana - nie po sztywnym czasie.
  let { statusText = "Wczytuję skrzynkę…" }: { statusText?: string } = $props();
</script>

<div class="splash">
  <div class="stage">
    <div class="glow"></div>
    <div class="ring"></div>
    <svg width="132" height="132" viewBox="0 0 64 64" fill="none" class="flower" aria-hidden="true">
      <g class="petal p-outer-left">
        <path d="M32 47C21.4 49 13.3 41 13.8 34.7c5.6-2.8 16.1 1.7 18.2 12.3Z" fill="currentColor" opacity=".48" />
      </g>
      <g class="petal p-outer-right">
        <path d="M32 47c2.1-10.6 12.6-15.1 18.2-12.3C50.7 41 42.6 49 32 47Z" fill="currentColor" opacity=".48" />
      </g>
      <g class="petal p-mid-left">
        <path d="M32 47c-11.9-3.8-16.3-16.6-12.6-22.7C26.6 24.4 35.1 34.9 32 47Z" fill="currentColor" opacity=".72" />
      </g>
      <g class="petal p-mid-right">
        <path d="M32 47c-3.1-12.1 5.4-22.6 12.6-22.7C48.3 30.4 43.9 43.2 32 47Z" fill="currentColor" opacity=".72" />
      </g>
      <g class="petal p-center">
        <path d="M32 47c-9.2-10.3-6.6-25.4 0-29.5 6.6 4.1 9.2 19.2 0 29.5Z" class="heart" />
      </g>
    </svg>
  </div>

  <div class="brand">
    <h1>lotusMail</h1>
    <p>based on Rust Engine</p>
  </div>

  <div class="status">
    <span class="dots">
      <span></span><span></span><span></span>
    </span>
    <span>{statusText}</span>
  </div>
</div>

<style>
  .splash {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: var(--paper);
  }

  .stage {
    position: relative;
    display: grid;
    place-items: center;
    width: 320px;
    height: 320px;
  }

  .glow {
    position: absolute;
    width: 300px;
    height: 300px;
    border-radius: 999px;
    background: radial-gradient(
      circle,
      color-mix(in oklab, var(--accent) 22%, transparent) 0%,
      color-mix(in oklab, var(--accent) 6%, transparent) 45%,
      transparent 70%
    );
    animation: lm-glow 1.1s cubic-bezier(0.22, 1, 0.36, 1) 0.12s both;
  }

  .ring {
    position: absolute;
    width: 210px;
    height: 210px;
    border-radius: 999px;
    border: 1px solid color-mix(in oklab, var(--accent) 50%, transparent);
    animation: lm-ring 1.15s cubic-bezier(0.22, 1, 0.36, 1) 0.62s both;
  }

  .flower {
    position: relative;
    color: var(--accent);
    animation: lm-breathe 3.2s ease-in-out 0.95s infinite;
  }
  .heart {
    fill: var(--flag);
  }

  /* Każdy płatek wyrasta z sercówki kwiatu, lekko obrócony na starcie. */
  .petal {
    transform-box: view-box;
    transform-origin: 32px 47px;
    animation: lm-petal 0.78s cubic-bezier(0.18, 0.9, 0.28, 1.06) both;
  }
  .p-center {
    --r: 0deg;
    animation-delay: 0.08s;
  }
  .p-mid-left {
    --r: 18deg;
    animation-delay: 0.26s;
  }
  .p-mid-right {
    --r: -18deg;
    animation-delay: 0.32s;
  }
  .p-outer-left {
    --r: 26deg;
    animation-delay: 0.46s;
  }
  .p-outer-right {
    --r: -26deg;
    animation-delay: 0.52s;
  }

  /* Znak firmowy siada tam, gdzie wcześniej kończył się kwiat - stąd
     ujemny margines; status przesuwa się pod niego. */
  .brand {
    margin-top: -30px;
    text-align: center;
  }
  .brand h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 30px;
    font-weight: 600;
    letter-spacing: -0.025em;
    color: var(--ink);
    animation: lm-fadeup 0.55s cubic-bezier(0.22, 1, 0.36, 1) 0.78s both;
  }
  .brand p {
    margin: 6px 0 0;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--muted);
    animation: lm-fadeup 0.55s cubic-bezier(0.22, 1, 0.36, 1) 0.9s both;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-top: 20px;
    font-size: 13px;
    color: var(--muted);
    animation: lm-fadeup 0.5s cubic-bezier(0.22, 1, 0.36, 1) 1.02s both;
  }
  .dots {
    display: flex;
    gap: 4px;
  }
  .dots span {
    width: 4px;
    height: 4px;
    border-radius: 999px;
    background: var(--accent);
    animation: lm-dot 1.1s ease-in-out infinite;
  }
  .dots span:nth-child(2) {
    animation-delay: 0.18s;
  }
  .dots span:nth-child(3) {
    animation-delay: 0.36s;
  }

  @keyframes lm-petal {
    from {
      opacity: 0;
      transform: scale(0.1) rotate(var(--r, 0deg));
    }
    55% {
      opacity: 1;
    }
    to {
      opacity: 1;
      transform: scale(1) rotate(0deg);
    }
  }
  @keyframes lm-glow {
    from {
      opacity: 0;
      transform: scale(0.4);
    }
    45% {
      opacity: 0.85;
    }
    to {
      opacity: 0.35;
      transform: scale(1);
    }
  }
  @keyframes lm-ring {
    from {
      opacity: 0;
      transform: scale(0.55);
    }
    30% {
      opacity: 0.45;
    }
    to {
      opacity: 0;
      transform: scale(1.35);
    }
  }
  @keyframes lm-fadeup {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  @keyframes lm-breathe {
    0%,
    100% {
      transform: scale(1);
    }
    50% {
      transform: scale(1.035);
    }
  }
  @keyframes lm-dot {
    0%,
    100% {
      opacity: 0.25;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .splash * {
      animation-duration: 0.01ms !important;
      animation-iteration-count: 1 !important;
    }
  }
</style>
