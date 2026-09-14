# ObsidianLog docs site

Source for the public documentation site at [Mintlify](https://mintlify.com).
Not a Mintlify page itself — this is a note for contributors editing the
site. `docs/adr` and `docs/grant` (elsewhere in this repo) are separate:
internal engineering/grant-tracking docs, not part of this public site.

## Previewing locally

```sh
npm i -g mintlify
cd docs-site
mintlify dev
```

Serves at `http://localhost:3000`. Requires Node.js ≥ 20.17.

## Structure

- `docs.json` — site config: theme, colors, and the navigation tree. Every
  page must be listed under `navigation.groups[].pages` to appear in the
  sidebar.
- Everything else is an `.mdx` page. Frontmatter (`title`, `description`) is
  required on every page.

## One-time setup (maintainers only)

This directory alone doesn't make the site live. Connecting it to Mintlify's
hosting is a one-time, account-level step that has to happen through the
Mintlify dashboard (not scriptable):

1. Sign up at [mintlify.com](https://mintlify.com) with GitHub OAuth.
2. Install the Mintlify GitHub App on this repository.
3. When prompted for the docs path, set it to `docs-site/` (not the repo
   root — that's where `docs.json` actually lives).

After that, every push to `main` that touches this directory auto-deploys —
no CI workflow needed on our side. `llms.txt`/`llms-full.txt`/per-page `.md`
exports are generated automatically once live, no extra config.
