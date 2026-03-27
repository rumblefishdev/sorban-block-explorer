---
url: 'https://github.com/cargo-lambda/cargo-lambda-cdk'
title: 'cargo-lambda-cdk: AWS CDK constructs for Rust Lambda functions'
fetched_date: 2026-03-26
task: '0006'
---

# Cargo Lambda CDK Construct

The `cargo-lambda-cdk` library provides AWS CDK constructs for building and deploying Rust Lambda functions using Cargo Lambda. It supports JavaScript/TypeScript, Go, and Python.

## Requirements

You need either Cargo Lambda (version 0.12.0+) or Docker installed to use this library.

## Installation

**JavaScript/TypeScript:**

```
npm i cargo-lambda-cdk
```

**Go:**
Add `github.com/cargo-lambda/cargo-lambda-cdk/cargolambdacdk` to imports.

**Python:**

```
pip install cargo-lambda-cdk
```

## RustFunction

Define a `RustFunction` with a manifest path:

```typescript
new RustFunction(stack, 'Rust function', {
  manifestPath: 'path/to/package/directory/with/Cargo.toml',
});
```

Project structure:

```
lambda-project
├── Cargo.toml
└── src
    └── main.rs
```

### Runtime Options

The default runtime is `provided.al2023`. Alternative: `provided.al2`

## RustExtension

Deploy as a Lambda layer:

```typescript
const extensionLayer = new RustExtension(this, 'Rust extension', {
  manifestPath: 'path/to/package/directory/with/Cargo.toml',
  architecture: Architecture.ARM_64,
});

new RustFunction(this, 'Rust function', {
  manifestPath: 'path/to/package/directory/with/Cargo.toml',
  layers: [extensionLayer],
});
```

## Remote Git Sources

Both constructs support cloning from Git repositories:

```typescript
new RustFunction(stack, 'Rust function', {
  gitRemote: 'https://github.com/your_user/your_repo',
  gitReference: 'branch',
  gitForceClone: true,
});
```

Options: valid git URLs, branch names, tags, or commit hashes.

## Bundling

### Local vs Docker Bundling

Local bundling uses installed Cargo Lambda. Docker bundling provides consistent Lambda-compatible environments.

### Environment Variables

```typescript
bundling: {
  environment: {
    HELLO: 'WORLD',
  },
}
```

### Build Profiles

```typescript
bundling: {
  profile: 'dev',
}
```

### Cargo Lambda Flags

```typescript
bundling: {
  cargoLambdaFlags: [
    '--target',
    'x86_64-unknown-linux-musl',
    '--debug',
  ],
}
```

### Docker Configuration

Force Docker bundling with `forcedDockerBundling: true`. Default image: `ghcr.io/cargo-lambda/cargo-lambda`

Custom image:

```typescript
bundling: {
  dockerImage: DockerImage.fromRegistry('your_docker_image'),
}
```

Mount volumes for Cargo cache:

```typescript
bundling: {
  dockerOptions: {
    volumes: [{
      hostPath: join(cargoHome, 'registry'),
      containerPath: '/usr/local/cargo/registry',
    }],
  },
}
```

### Command Hooks

Run commands before/after bundling:

```typescript
bundling: {
  commandHooks: {
    beforeBundling(inputDir, outputDir) {
      return ['cargo test'];
    },
  },
}
```

Available hooks: `beforeBundling`, `afterBundling`

## Asset Hash Type

Adjust the `assetHashType` parameter:

- `AssetHashType.OUTPUT` (default): hash based on compiled binary
- `AssetHashType.SOURCE`: hash based on source folder

## License

MIT
