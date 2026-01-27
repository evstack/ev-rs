# Evolve Documentation (Vocs)

Documentation site built with [Vocs](https://vocs.dev).

## Development

```bash
# Install dependencies
bun install

# Start dev server
bun run dev
```

Open http://localhost:5173

## Build

```bash
# Build for production
bun run build

# Preview production build
bun run preview
```

## Structure

```
website/
├── docs/
│   ├── pages/           # MDX content
│   │   ├── index.mdx    # Homepage
│   │   ├── getting-started/
│   │   ├── concepts/
│   │   ├── modules/
│   │   ├── reference/
│   │   ├── ethereum/
│   │   └── operating/
│   └── public/          # Static assets
├── vocs.config.ts       # Vocs configuration
├── package.json
└── tsconfig.json
```

## Deployment

Build outputs to `dist/`. Deploy to any static hosting.

### Cloudflare Pages

**Via Git integration (recommended):**
1. Connect repo to Cloudflare Pages
2. Set build command: `bun run build`
3. Set output directory: `dist`
4. Set root directory: `website`
5. Leave deploy command blank

**Via CLI:**
```bash
bun run build
npx wrangler pages deploy dist --project-name=evolve-docs
```

### Other Platforms

- Vercel: `bunx vercel`
- Netlify: Point to `dist/` directory
- GitHub Pages: Use GitHub Actions
