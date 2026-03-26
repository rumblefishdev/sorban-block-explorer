---
url: 'https://docs.aws.amazon.com/lambda/latest/dg/rust-package.html'
title: 'Deploy Rust Lambda functions with .zip file archives'
fetched_date: 2026-03-26
task: '0006'
---

# Deploy Rust Lambda functions with .zip file archives

This page describes how to compile your Rust function and deploy the compiled binary to AWS Lambda using [Cargo Lambda](https://www.cargo-lambda.info/guide/what-is-cargo-lambda.html). It also shows how to deploy the compiled binary with the AWS CLI and the AWS SAM CLI.

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install)
- [AWS CLI version 2](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html)

## Building Rust Functions

### 1. Install Cargo Lambda

```
cargo install cargo-lambda
```

### 2. Create the package structure

```
cargo lambda new my-function
```

This creates basic function code in `src/main.rs`.

### 3. Compile the function

```
cargo lambda build --release
```

For AWS Graviton2 (ARM):

```
cargo lambda build --release --arm64
```

### 4. Configure AWS credentials

```
aws configure
```

## Deploying with Cargo Lambda

```
cargo lambda deploy my-function
```

This creates an execution role and creates the Lambda function. To specify an existing execution role, use the `--iam-role` flag.

## Deploying with AWS CLI

### 1. Build the .zip deployment package

```
cargo lambda build --release --output-format zip
```

### 2. Create the Lambda function

```
aws lambda create-function \
   --function-name my-function \
   --runtime provided.al2023 \
   --role arn:aws:iam::111122223333:role/lambda-role \
   --handler rust.handler \
   --zip-file fileb://target/lambda/my-function/bootstrap.zip
```

Key parameters:

- `--runtime provided.al2023` — OS-only runtime for compiled binaries and custom runtimes
- `--handler rust.handler` — handler name for Rust binaries

## Deploying with AWS SAM CLI

### 1. Create a SAM template

```yaml
AWSTemplateFormatVersion: '2010-09-09'
Transform: AWS::Serverless-2016-10-31
Description: SAM template for Rust binaries
Resources:
  RustFunction:
    Type: AWS::Serverless::Function
    Properties:
      CodeUri: target/lambda/my-function/
      Handler: rust.handler
      Runtime: provided.al2023
Outputs:
  RustFunction:
    Description: 'Lambda Function ARN'
    Value: !GetAtt RustFunction.Arn
```

### 2. Compile the function

```
cargo lambda build --release
```

### 3. Deploy

```
sam deploy --guided
```

## Invoking the Function

### With Cargo Lambda

```
cargo lambda invoke --remote --data-ascii '{"command": "Hello world"}' my-function
```

### With AWS CLI

```
aws lambda invoke --function-name my-function --cli-binary-format raw-in-base64-out --payload '{"command": "Hello world"}' /tmp/out.txt
```

Note: `--cli-binary-format raw-in-base64-out` is required for AWS CLI version 2. To make it the default: `aws configure set cli-binary-format raw-in-base64-out`.
