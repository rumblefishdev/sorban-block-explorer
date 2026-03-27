---
url: 'https://docs.aws.amazon.com/cdk/v2/guide/best-practices.html'
title: 'Best practices for developing and deploying cloud infrastructure with the AWS CDK'
fetched_date: 2026-03-26
task: '0006'
---

# Best practices for developing and deploying cloud infrastructure with the AWS CDK

With the AWS CDK, developers or administrators can define their cloud infrastructure by using a supported programming language. CDK applications should be organized into logical units, such as API, database, and monitoring resources, and optionally have a pipeline for automated deployments. The logical units should be implemented as constructs including the following:

- Infrastructure (such as Amazon S3 buckets, Amazon RDS databases, or an Amazon VPC network)
- Runtime code (such as AWS Lambda functions)
- Configuration code

Stacks define the deployment model of these logical units.

The AWS CDK reflects careful consideration of the needs of our customers and internal teams and of the failure patterns that often arise during the deployment and ongoing maintenance of complex cloud applications. Failures are often related to "out-of-band" changes to an application that aren't fully tested. Therefore, the AWS CDK models your entire application in code — not only business logic but also infrastructure and configuration — so that proposed changes can be carefully reviewed, comprehensively tested, and fully rolled back if something goes wrong.

At deployment time, the AWS CDK synthesizes a cloud assembly that contains:

- AWS CloudFormation templates that describe your infrastructure in all target environments
- File assets that contain your runtime code and their supporting files

## Organization Best Practices

It's a best practice to have a team of experts (Cloud Center of Excellence, CCoE) responsible for training and guiding the rest of the company as they adopt the CDK. This team sets standards and policies for cloud infrastructure, and creates a "landing zone" — a pre-configured, secure, scalable, multi-account AWS environment.

Development teams should be able to use their own accounts for testing. Using CDK Pipelines, AWS CDK applications can then be deployed via a CI/CD account to testing, integration, and production environments (each isolated in its own AWS Region or account).

## Coding Best Practices

### Start simple and add complexity only when you need it

Add complexity only when your requirements dictate a more complicated solution. With the AWS CDK, you can refactor your code as necessary to support new requirements.

### Align with the AWS Well-Architected Framework

An AWS CDK application maps to a _component_ as defined by the AWS Well-Architected Framework. You can also create and share components as reusable code libraries through artifact repositories such as AWS CodeArtifact.

### Every application starts with a single package in a single repository

A single package is the entry point of your AWS CDK app. Use additional packages for constructs that you use in more than one application.

Avoid putting multiple applications in the same repository, especially when using automated deployment pipelines. Changes to one application would trigger deployment of the others.

### Move code into repositories based on code lifecycle or team ownership

When packages begin to be used in multiple applications, move them to their own repository. To consume packages across repository boundaries, you need a private package repository (similar to NPM, PyPi, or Maven Central). CodeArtifact can host packages for most popular programming languages.

### Infrastructure and runtime code live in the same package

The AWS CDK bundles runtime assets like Lambda functions and Docker images and deploys them alongside your infrastructure. Combine the code that defines your infrastructure and the code that implements your runtime logic into a single construct.

## Construct Best Practices

### Model with constructs, deploy with stacks

Stacks are the unit of deployment: everything in a stack is deployed together. Represent each logical unit as a `Construct`, not as a `Stack`. Use stacks only to describe how your constructs should be composed and connected for your various deployment scenarios.

### Configure with properties and methods, not environment variables

Environment variable lookups inside constructs and stacks are a common anti-pattern. Both constructs and stacks should accept a properties object to allow for full configurability completely in code. Environment variable lookups should be limited to the top level of an AWS CDK app.

### Unit test your infrastructure

Avoid network lookups during synthesis and model all your production stages in code. If any single commit always results in the same generated template, you can trust unit tests to confirm that the generated templates look the way you expect.

### Don't change the logical ID of stateful resources

Changing the logical ID of a resource results in the resource being replaced with a new one at the next deployment. For stateful resources like databases and S3 buckets, or persistent infrastructure like an Amazon VPC, this is seldom what you want. Write unit tests that assert the logical IDs of stateful resources remain static.

### Constructs aren't enough for compliance

Use AWS features such as service control policies and permission boundaries to enforce security guardrails at the organization level. Use Aspects or tools like CloudFormation Guard to make assertions about security properties before deployment.

## Application Best Practices

### Make decisions at synthesis time

Make all decisions (such as which construct to instantiate) in your AWS CDK application using your programming language's `if` statements and other features. Avoid using CloudFormation `Conditions`, `{ Fn::If }`, and `Parameters` at deployment time.

### Use generated resource names, not physical names

If you hardcode a table name or bucket name, you can't deploy that infrastructure twice in the same account. Specify as few names as possible — let the AWS CDK generate them. Pass the generated names as environment variables into Lambda functions or reference them as `table.tableName`.

### Define removal policies and log retention

The AWS CDK defaults to retaining everything you create. In production environments this can result in storing large amounts of data you don't need. Consider carefully what you want removal policies and log retention to be for each production resource.

### Separate your application into multiple stacks as dictated by deployment requirements

Guidelines:

- Keep as many resources in the same stack as possible unless you know you want them separated.
- Keep stateful resources (like databases) in a separate stack from stateless resources; enable termination protection on the stateful stack.
- Stateful resources are more sensitive to construct renaming — renaming leads to resource replacement.

### Commit `cdk.context.json` to avoid non-deterministic behavior

The AWS CDK includes a mechanism called _context providers_ to record a snapshot of non-deterministic values (e.g., AZ lists, AMI IDs). This allows future synthesis operations to produce exactly the same template as they did when first deployed. The result of `.fromLookup()` calls is cached in `cdk.context.json` — commit this to version control.

### Let the AWS CDK manage roles and security groups

Use the `grants` property and convenience methods to create IAM roles that grant access to one resource by another using minimally scoped permissions. Example:

```typescript
bucket.grantRead(myLambda);
```

This single line adds a policy to the Lambda function's role with minimal required permissions.

### Model all production stages in code

Create a stack for your production environment, and a separate stack for each of your other stages. Put the configuration values for each stack in the code. Use Secrets Manager and Systems Manager Parameter Store for sensitive values.

### Measure everything

Create metrics, alarms, and dashboards to measure all aspects of your deployed resources. Record business metrics and use those measurements to automate deployment decisions like rollbacks. Most L2 constructs have convenience methods to help you create metrics (e.g., `dynamodb.Table.metricUserErrors()`).
