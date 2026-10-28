<!-- Generated from tmtroot/readme.tmt. Edit that, then `tomet export .`. -->

# Tomet CMS

Content management system for the Tomet ecosystem.

## Overview

`tomet-cms` provides Git-backed content management capabilities for Tomet workspaces.

## Usage

```bash
tomet-cms [path]              # open CMS for path (defaults to ".")
```

## Workspace Layout

- `app` (`tomet-cms`) -- Main CLI binary.

## Building and Testing

### Build workspace

```bash
just build
```

### Run tests

```bash
just test
```

### Check code

```bash
just check
just clippy
```

### Documentation

```bash
just docs        # regenerate Markdown from tmtroot/
just docs-check  # verify docs match exports
```

