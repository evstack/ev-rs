import { defineConfig } from 'vocs'

export default defineConfig({
  title: 'Evolve',
  description: 'Modular Rust SDK for building blockchain applications with Ethereum compatibility',
  logoUrl: '/logo.svg',
  iconUrl: '/favicon.ico',

  socials: [
    {
      icon: 'github',
      link: 'https://github.com/evolvesdk/evolve',
    },
  ],

  sidebar: [
    {
      text: 'Introduction',
      link: '/',
    },
    {
      text: 'Getting Started',
      collapsed: false,
      items: [
        { text: 'Installation', link: '/getting-started/installation' },
        { text: 'Quickstart', link: '/getting-started/quickstart' },
        { text: 'Running the Testapp', link: '/getting-started/testapp' },
      ],
    },
    {
      text: 'Core Concepts',
      collapsed: false,
      items: [
        { text: 'Accounts', link: '/concepts/accounts' },
        { text: 'State Transition Function', link: '/concepts/stf' },
        { text: 'Determinism', link: '/concepts/determinism' },
        { text: 'Concurrency Model', link: '/concepts/concurrency' },
      ],
    },
    {
      text: 'Building Modules',
      collapsed: false,
      items: [
        { text: 'Architecture', link: '/modules/architecture' },
        { text: 'Creating Modules', link: '/modules/creating' },
        { text: 'Storage', link: '/modules/storage' },
        { text: 'Error Handling', link: '/modules/errors' },
        { text: 'Testing', link: '/modules/testing' },
      ],
    },
    {
      text: 'SDK Reference',
      collapsed: true,
      items: [
        { text: 'Macros', link: '/reference/macros' },
        { text: 'Collections', link: '/reference/collections' },
        { text: 'Standards', link: '/reference/standards' },
        { text: 'Pre-built Modules', link: '/reference/prebuilt' },
        { text: 'Schema Introspection', link: '/reference/schema-introspection' },
        { text: 'gRPC API', link: '/reference/grpc' },
      ],
    },
    {
      text: 'Ethereum Compatibility',
      collapsed: true,
      items: [
        { text: 'Transactions', link: '/ethereum/transactions' },
        { text: 'JSON-RPC', link: '/ethereum/json-rpc' },
        { text: 'Wallet Integration', link: '/ethereum/wallets' },
      ],
    },
    {
      text: 'Operating',
      collapsed: true,
      items: [
        { text: 'Server Configuration', link: '/operating/server' },
        { text: 'Production Deployment', link: '/operating/production' },
      ],
    },
  ],

  theme: {
    accentColor: '#6366f1', // indigo
  },
})
