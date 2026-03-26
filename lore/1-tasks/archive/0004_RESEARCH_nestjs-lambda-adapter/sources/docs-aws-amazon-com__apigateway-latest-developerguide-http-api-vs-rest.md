---
url: 'https://docs.aws.amazon.com/apigateway/latest/developerguide/http-api-vs-rest.html'
title: 'Choose between REST APIs and HTTP APIs - Amazon API Gateway'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# Choose between REST APIs and HTTP APIs

REST APIs and HTTP APIs are both RESTful API products. REST APIs support more features than HTTP APIs, while HTTP APIs are designed with minimal features so that they can be offered at a lower price. Choose REST APIs if you need features such as API keys, per-client throttling, request validation, AWS WAF integration, or private API endpoints. Choose HTTP APIs if you don't need the features included with REST APIs.

The following sections summarize core features that are available in REST APIs and HTTP APIs. When necessary, additional links are provided to navigate between the REST API and HTTP API sections of the API Gateway Developer Guide.

## Endpoint type

The endpoint type refers to the endpoint that API Gateway creates for your API.

| Feature        | REST API | HTTP API |
| -------------- | -------- | -------- |
| Edge-optimized | Yes      | No       |
| Regional       | Yes      | Yes      |
| Private        | Yes      | No       |

## Security

API Gateway provides a number of ways to protect your API from certain threats, like malicious actors or spikes in traffic.

| Feature                                 | REST API | HTTP API |
| --------------------------------------- | -------- | -------- |
| Mutual TLS authentication               | Yes      | Yes      |
| Certificates for backend authentication | Yes      | No       |
| AWS WAF                                 | Yes      | No       |

## Authorization

API Gateway supports multiple mechanisms for controlling and managing access to your API.

| Feature                                          | REST API | HTTP API |
| ------------------------------------------------ | -------- | -------- |
| IAM                                              | Yes      | Yes      |
| Resource policies                                | Yes      | No       |
| Amazon Cognito                                   | Yes      | Yes ¹    |
| Custom authorization with an AWS Lambda function | Yes      | Yes      |
| JSON Web Token (JWT) ²                           | No       | Yes      |

¹ You can use Amazon Cognito with a JWT authorizer.

² You can use a Lambda authorizer to validate JWTs for REST APIs.

## API management

Choose REST APIs if you need API management capabilities such as API keys and per-client rate limiting.

| Feature                     | REST API | HTTP API |
| --------------------------- | -------- | -------- |
| Custom domains              | Yes      | Yes      |
| API keys                    | Yes      | No       |
| Per-client rate limiting    | Yes      | No       |
| Per-client usage throttling | Yes      | No       |
| Developer portal            | Yes      | No       |

## Development

| Feature                          | REST API | HTTP API |
| -------------------------------- | -------- | -------- |
| CORS configuration               | Yes      | Yes      |
| Test invocations                 | Yes      | No       |
| Caching                          | Yes      | No       |
| User-controlled deployments      | Yes      | Yes      |
| Automatic deployments            | No       | Yes      |
| Custom gateway responses         | Yes      | No       |
| Canary release deployments       | Yes      | No       |
| Request validation               | Yes      | No       |
| Request parameter transformation | Yes      | Yes      |
| Request body transformation      | Yes      | No       |

## Monitoring

| Feature                             | REST API | HTTP API |
| ----------------------------------- | -------- | -------- |
| Amazon CloudWatch metrics           | Yes      | Yes      |
| Access logs to CloudWatch Logs      | Yes      | Yes      |
| Access logs to Amazon Data Firehose | Yes      | No       |
| Execution logs                      | Yes      | No       |
| AWS X-Ray tracing                   | Yes      | No       |

## Integrations

| Feature                                              | REST API | HTTP API |
| ---------------------------------------------------- | -------- | -------- |
| Public HTTP endpoints                                | Yes      | Yes      |
| AWS services                                         | Yes      | Yes      |
| AWS Lambda functions                                 | Yes      | Yes      |
| Private integrations with Network Load Balancers     | Yes      | Yes      |
| Private integrations with Application Load Balancers | Yes      | Yes      |
| Private integrations with AWS Cloud Map              | No       | Yes      |
| Mock integrations                                    | Yes      | No       |
| Response streaming                                   | Yes      | No       |
