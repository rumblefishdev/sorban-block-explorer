# AWS Builder's Library: Making Retries Safe with Idempotent APIs

**Source:** https://aws.amazon.com/builders-library/making-retries-safe-with-idempotent-APIs/
**Fetched:** 2026-03-27

---

By Malcolm Featonby | [PDF](https://d1.awsstatic.com/builderslibrary/pdfs/making-retries-safe-with-idempotent-apis-malcolm-featonby.pdf)

At Amazon, complex operations are often decomposed into a controlling process that makes calls to smaller services, each handling one part of the workflow. For example, launching an EC2 instance involves calls to services for placement decisions, creating EBS volumes, creating network interfaces, and provisioning a virtual machine. When service calls fail, retrying until success is often the simplest solution.

As explained in related guidance, many transient or random faults can be overcome through simple retries. This pattern is so effective that AWS SDK implementations include default retry behavior for requests failing due to network issues, server-side faults, or service rate limiting. This reduces the number of edge cases clients must handle and decreases boilerplate code.

However, retrying safely depends on a key assumption: "an operation can be retried without any side effects." The goal is ensuring the operation occurs only once, even if called multiple times. For instance, retrying a failed EBS volume creation shouldn't result in two volumes. AWS leverages idempotent API operations to mitigate undesirable retry side effects while simplifying client-side code.

## Retrying and Side Effects

### Retrying and the Potential for Undesirable Side Effects

Consider a customer using the Amazon EC2 RunInstances API to run a singleton workload requiring "at most one" EC2 instance at any time. If the provisioning process receives no response due to network timeout, uncertainty arises about whether the workload is running.

![Making retries safe diagram 1](https://d1.awsstatic.com/builderslibrary/architecture-images/making-retries-safe-1.56760d1d28759735e71c26243f3c1d190329c2a0.png)

Simply retrying could result in multiple workloads, potentially causing serious issues. The provisioning process must perform reconciliation to determine if the workload exists—significant overhead for a relatively infrequent edge case. Even with reconciliation, uncertainty remains: was the resource created by this process or another?

## Reducing Client Complexity

### Reducing Client Complexity with Idempotent API Design

To allow callers to retry operations safely, services must make them idempotent. An idempotent operation allows request retransmission or retries without additional side effects—highly beneficial in distributed systems.

Client code can be significantly simplified by providing a contract allowing the assumption that any non-validation error can be overcome by retrying. However, this introduces service implementation complexity: how do we identify whether a request duplicates a previous one?

Various approaches exist to infer request duplication. For example, deriving a synthetic token from request parameters by hashing them might suggest that identical requests from the same caller are duplicates. However, this approach has limitations. Two identical DynamoDB table creation requests might reasonably be considered duplicates, but two identical EC2 instance launch requests might legitimately represent a customer wanting two instances.

AWS's preferred approach incorporates a unique caller-provided client request identifier into the API contract. Requests from the same caller with the same identifier are considered duplicates. This approach reduces unexpected outcomes by allowing customers to express intent through API semantics. The identifier is auditable in logs like AWS CloudTrail and can label created resources, enabling customers to identify resources from any given request. In the EC2 API, this identifier is called the ClientToken.

The following diagram shows a request/response flow using a unique client request identifier in an idempotent retry scenario:

![Making retries safe diagram 2](https://d1.awsstatic.com/builderslibrary/architecture-images/making-retries-safe-2.75b41b5b444aa53dbf460396495e3f30b5c6af49.png)

In this example, the customer requests resource creation with a unique identifier. The service checks whether it has seen this identifier. If not, it processes the request and creates an idempotent "session" keyed to the customer identifier and unique client request identifier. If a subsequent request arrives with the same identifier, the service knows it has already processed it and takes appropriate action.

Critically, the process combining idempotent token recording and all mutating operations must meet ACID (Atomic, Consistent, Isolated, Durable) properties. This ensures all-or-nothing processing, avoiding scenarios where the token is recorded but resources fail to create, or resources are created but token recording fails.

The diagram shows preparing a semantically equivalent response when the request has been seen before. While not strictly required for basic idempotency, this approach offers important benefits. If the first request succeeds but the response doesn't reach the caller, a retry with the same identifier might otherwise return ResourceAlreadyExists. Though technically idempotent (no side effect), this creates uncertainty—was the resource created by this request or an earlier one? Additionally, it complicates introducing default retry behavior, as the different return code changes client execution flow, creating a side effect from the client's perspective.

## Semantic Equivalence and Support for Default Retry Strategies

An alternative is delivering a semantically equivalent response for the same unique identifier over some interval. Any retry request from the same caller with the same identifier receives the same meaningful response as the first successful request. This approach offers useful properties, especially for improving customer experience by safely retrying operations experiencing server-side faults through default retry policies.

This idempotency with semantically equivalent responses and automated retry logic appears in the Amazon EC2 RunInstances API operation with the AWS CLI. The AWS CLI (like the AWS SDK) [supports a default retry policy](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-retries.html). Example:

```
$ aws ec2 run-instances --image-id ami-04fcd96153cb57194 --instance-type t2.micro
```

```json
{
  "Instances": [
    {
      "Monitoring": {
        "State": "disabled"
      },
      "StateReason": {
        "Message": "pending",
        "Code": "pending"
      },
      "State": {
        "Code": 0,
        "Name": "pending"
      },
      "InstanceId": "i-xxxxxxxxxxxxxxxxx",
      "ImageId": "ami-04fcd96153cb57194"
    }
  ]
}
```
