---
url: 'https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-task-storage.html'
title: 'Fargate task ephemeral storage for Amazon ECS'
fetched_date: 2026-03-25
task_id: '0001'
---

# Fargate task ephemeral storage for Amazon ECS

**Source:** https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-task-storage.html
**Fetched:** 2026-03-25

---

When provisioned, each Amazon ECS task hosted on Linux containers on AWS Fargate receives the following ephemeral storage for bind mounts. This can be mounted and shared among containers that use the `volumes`, `mountPoints`, and `volumesFrom` parameters in the task definition. This isn't supported for Windows containers on AWS Fargate.

## Fargate Linux container platform versions

### Version 1.4.0 or later

By default, Amazon ECS tasks that are hosted on Fargate using platform version `1.4.0` or later receive a minimum of **20 GiB** of ephemeral storage. The total amount of ephemeral storage can be increased, up to a maximum of **200 GiB**. You can do this by specifying the `ephemeralStorage` parameter in your task definition.

The pulled, compressed, and the uncompressed container image for the task is stored on the ephemeral storage. To determine the total amount of ephemeral storage your task has to use, you must subtract the amount of storage your container image uses from the total amount of ephemeral storage your task is allocated.

For tasks that use platform version `1.4.0` or later that are launched on May 28, 2020 or later, the ephemeral storage is encrypted with an AES-256 encryption algorithm. This algorithm uses an AWS owned encryption key, or you can create your own customer managed key. For more information, see [Customer managed keys for AWS Fargate ephemeral storage](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-storage-encryption.html).

For tasks that use platform version `1.4.0` or later that are launched on November 18, 2022 or later, the ephemeral storage usage is reported through the task metadata endpoint. Your applications in your tasks can query the task metadata endpoint version 4 to get their ephemeral storage reserved size and the amount used.

Additionally, the ephemeral storage reserved size and the amount used are sent to Amazon CloudWatch Container Insights if you turn on Container Insights.

> **Note**
>
> Fargate reserves space on disk. It is only used by Fargate. You aren't billed for it. It isn't shown in these metrics. However, you can see this additional storage in other tools such as `df`.

### Version 1.3.0 or earlier

For Amazon ECS on Fargate tasks that use platform version `1.3.0` or earlier, each task receives the following ephemeral storage.

- 10 GB of Docker layer storage

  > **Note:** This amount includes both compressed and uncompressed container image artifacts.

- An additional 4 GB for volume mounts. This can be mounted and shared among containers that use the `volumes`, `mountPoints`, and `volumesFrom` parameters in the task definition.

## Fargate Windows container platform versions

### Version 1.0.0 or later

By default, Amazon ECS tasks that are hosted on Fargate using platform version `1.0.0` or later receive a minimum of **20 GiB** of ephemeral storage. The total amount of ephemeral storage can be increased, up to a maximum of **200 GiB**. You can do this by specifying the `ephemeralStorage` parameter in your task definition.

The pulled, compressed, and the uncompressed container image for the task is stored on the ephemeral storage. To determine the total amount of ephemeral storage that your task has to use, you must subtract the amount of storage that your container image uses from the total amount of ephemeral storage your task is allocated.

For more information, see [Use bind mounts with Amazon ECS](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/bind-mounts.html).

---

## Key Takeaways

| Platform         | Min storage                     | Max storage |
| ---------------- | ------------------------------- | ----------- |
| Linux >= 1.4.0   | 20 GiB                          | 200 GiB     |
| Linux <= 1.3.0   | 10 GB (layers) + 4 GB (volumes) | fixed       |
| Windows >= 1.0.0 | 20 GiB                          | 200 GiB     |

- Storage is ephemeral — lost when the task stops
- Container image storage counts against the ephemeral storage limit
- Use `ephemeralStorage` parameter in task definition to increase beyond 20 GiB
- Storage usage observable via task metadata endpoint (v4) and CloudWatch Container Insights
