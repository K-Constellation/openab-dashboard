# OpenAB Dashboard — Frontend Design

## Stack

| Layer | Technology | Loaded From |
|-------|-----------|-------------|
| Interactivity | HTMX 2.x | CDN |
| Charts | Chart.js 4.x | CDN |
| Styling | Tailwind CSS 3.x | CDN (play) |
| Icons | Heroicons (inline SVG) | Embedded |

**No build step. No npm. No bundler.** Just HTML files embedded in the Rust binary.

## Theme

Dark theme with neon accent colors (inspired by the reference screenshot):

```css
:root {
    --bg-primary: #0a0e1a;       /* Deep dark blue */
    --bg-secondary: #111827;     /* Card background */
    --bg-card: #1a2035;          /* Elevated card */
    --text-primary: #e2e8f0;     /* Light gray */
    --text-secondary: #94a3b8;   /* Muted */
    --accent-cyan: #22d3ee;      /* Primary accent */
    --accent-orange: #fb923c;    /* Secondary accent */
    --accent-pink: #f472b6;      /* Tertiary accent */
    --accent-green: #4ade80;     /* Success */
    --accent-red: #f87171;       /* Warning/danger */
}
```

## Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ HEADER                                                               │
│ ┌──────────────────┐  ┌──────────┐              Last refresh: HH:MM │
│ │ Time: [Month ▾]  │  │ 🔄 Refresh│                                 │
│ └──────────────────┘  └──────────┘                                  │
├──────────────────────────────────────────────────────────────────────┤
│ PROVIDER TABS                                                        │
│ [ 🔥 All ]  [ Kiro ]  [ Antigravity ]                               │
├──────────────────────────────────────────────────────────────────────┤
│ STAT CARDS (row of 4)                                                │
│ ┌────────────┐ ┌────────────┐ ┌────────────────┐ ┌────────────────┐ │
│ │ Total Used │ │ Today      │ │ Top Agent      │ │ Days to Reset  │ │
│ │   342.5    │ │   45.2     │ │ masami         │ │     18         │ │
│ │  credits   │ │  credits   │ │                │ │                │ │
│ └────────────┘ └────────────┘ └────────────────┘ └────────────────┘ │
├──────────────────────────────────────────────────────────────────────┤
│ DAILY CHART                                                          │
│ 📊 Daily Usage                                                       │
│                                                                      │
│  2000│                                                               │
│      │  ███                         ███  ███                         │
│  1500│  ███                    ███  ███  ███                         │
│      │  ███  ███              ███  ███  ███                         │
│  1000│  ███  ███  ███        ███  ███  ███  ███                     │
│      │  ███  ███  ███  ███  ███  ███  ███  ███                     │
│   500│  ███  ███  ███  ███  ███  ███  ███  ███  ███                 │
│      │  ███  ███  ███  ███  ███  ███  ███  ███  ███                 │
│     0└──────────────────────────────────────────────────             │
│       07/01 07/02 07/03 07/04 07/05 07/06 07/07 07/08               │
│                                                                      │
│       ■ masami  ■ chloe  ■ misaki                                   │
├──────────────────────────────────────────────────────────────────────┤
│ LEADERBOARD                                                          │
│ 🏆 Agent Ranking — 07/01–07/14                                       │
│                                                                      │
│  🥇 masami (kiro)                                         65%        │
│     ████████████████████████████████░░░░░░░░░░░  520.3 credits      │
│                                              ▶ 1.56M tokens  🔧 245 │
│                                                                      │
│  🥈 chloe (kiro)                                          41%        │
│     █████████████████████░░░░░░░░░░░░░░░░░░░░░  410.8 credits      │
│                                              ▶ 1.23M tokens  🔧 198 │
│                                                                      │
│  🥉 misaki (antigravity)                                  28%        │
│     ██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░  — tokens only       │
│                                              ▶ 670K tokens   🔧 89  │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

## HTMX Interactions

### Auto-refresh (every 60s)
```html
<div hx-get="/api/summary?days=14"
     hx-trigger="load, every 60s"
     hx-swap="innerHTML">
</div>
```

### Time range filter
```html
<select hx-get="/api/summary"
        hx-target="#chart-container"
        hx-include="[name='days']"
        name="days">
    <option value="7">This Week</option>
    <option value="14" selected>2 Weeks</option>
    <option value="30">This Month</option>
</select>
```

### Provider tab switching
```html
<button hx-get="/api/summary?provider=kiro"
        hx-target="#main-content"
        class="tab active">
    Kiro
</button>
```

## Chart.js Configuration

### Daily Usage (Stacked Bar)
```javascript
new Chart(ctx, {
    type: 'bar',
    data: {
        labels: dates,
        datasets: [
            { label: 'masami', data: [...], backgroundColor: '#22d3ee' },
            { label: 'chloe', data: [...], backgroundColor: '#fb923c' },
            { label: 'misaki', data: [...], backgroundColor: '#4ade80' }
        ]
    },
    options: {
        scales: { x: { stacked: true }, y: { stacked: true } },
        plugins: { legend: { position: 'bottom' } }
    }
});
```

## Responsive Design

- Desktop-first (primary use case: viewed on your-server or laptop)
- Mobile-friendly with Tailwind breakpoints
- Cards stack vertically on small screens

## File Structure

```
static/
├── index.html          ← Main dashboard page
├── css/
│   └── dashboard.css   ← Custom styles (minimal, mostly Tailwind)
└── js/
    └── dashboard.js    ← Chart initialization + HTMX config
```

All files embedded into the Rust binary at compile time via `rust-embed`.
