# Claw

> Turn any website into a CLI — with native browser precision.

Rust-based CLI tool that transforms websites into deterministic command-line interfaces. Compatible with [opencli](https://github.com/jackwener/opencli) YAML adapters, powered by CDP-level browser control.

## Why Claw

opencli proved the concept: pre-built adapters beat LLM-driven browsing for cost, speed, and reliability. But its Node.js runtime and JS event simulation create real limitations:

- React/Vue SPAs don't respond to `el.click()` — submit buttons, modals, tab switches silently fail
- Node.js 20+ runtime dependency (~800MB) for a CLI tool
- ~500ms cold start from Node.js + JS compilation

Claw keeps what works (YAML adapter ecosystem, 5-tier auth strategy, AI-powered discovery) and fixes what doesn't (browser control precision, distribution, startup).

## Core Difference

```
opencli:  JS dispatchEvent → React ignores it → silent failure
claw:     CDP Input.dispatchMouseEvent → browser-native → works everywhere
```

## Architecture

```
┌─────────────────────────────────────┐
│  Layer 3: Adapters (YAML / Lua)     │  Hot-loadable, opencli-compatible
├─────────────────────────────────────┤
│  Layer 2: Pipeline Engine (Rust)    │  navigate/wait/evaluate/map/click/type
├─────────────────────────────────────┤
│  Layer 1: CDP Client (Rust)         │  Native browser control
└─────────────────────────────────────┘
```

## Quick Start

```bash
claw list                              # See all commands
claw bilibili hot --limit 5            # Browser command
claw jimeng generate "a cat"           # Trigger image generation
claw xiaohongshu publish "content" \
  --title "title" --images cover.webp  # Publish note (actually works)
```

## Status

Early development — CDP client, pipeline engine, and YAML adapters working. Xiaohongshu publish verified end-to-end.

## License

MIT
