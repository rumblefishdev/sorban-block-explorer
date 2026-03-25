---
url: 'https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task-cpu-memory-error.html'
title: 'Troubleshoot Amazon ECS task definition invalid CPU or memory errors'
fetched_date: 2026-03-25
task_id: '0001'
---

# Troubleshoot Amazon ECS task definition invalid CPU or memory errors

**Source:** https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task-cpu-memory-error.html
**Fetched:** 2026-03-25

---

## Error Message

When registering a task definition using the Amazon ECS API or AWS CLI, if you specify an invalid `cpu` or `memory` value, the following error is returned:

```
An error occurred (ClientException) when calling the RegisterTaskDefinition operation: Invalid 'cpu' setting for task.
```

When using Terraform, the following error might be returned:

```
Error: ClientException: No Fargate configuration exists for given values.
```

## Resolution

To resolve this issue, you must specify a supported value for the task CPU and memory in your task definition. The `cpu` value can be expressed in CPU units or vCPUs in a task definition. It's converted to an integer indicating the CPU units when the task definition is registered. The `memory` value can be expressed in MiB or GB in a task definition. It's converted to an integer indicating the MiB when the task definition is registered.

## Fargate Task CPU and Memory Combinations

For task definitions that specify `FARGATE` for the `requiresCompatibilities` parameter (even if `EC2` is also specified), you must use one of the values in the following table:

| CPU value       | Memory value                                | Operating systems supported for AWS Fargate |
| --------------- | ------------------------------------------- | ------------------------------------------- |
| 256 (.25 vCPU)  | 512 MiB, 1 GB, 2 GB                         | Linux                                       |
| 512 (.5 vCPU)   | 1 GB, 2 GB, 3 GB, 4 GB                      | Linux                                       |
| 1024 (1 vCPU)   | 2 GB, 3 GB, 4 GB, 5 GB, 6 GB, 7 GB, 8 GB    | Linux, Windows                              |
| 2048 (2 vCPU)   | Between 4 GB and 16 GB in 1 GB increments   | Linux, Windows                              |
| 4096 (4 vCPU)   | Between 8 GB and 30 GB in 1 GB increments   | Linux, Windows                              |
| 8192 (8 vCPU)   | Between 16 GB and 60 GB in 4 GB increments  | Linux                                       |
| 16384 (16 vCPU) | Between 32 GB and 120 GB in 8 GB increments | Linux                                       |

> **Note:** The 8192 (8 vCPU) and 16384 (16 vCPU) options require Linux platform `1.4.0` or later.

The memory values in the JSON file are specified in MiB. You can convert the GB value to MiB by multiplying the value by 1024. For example: 1 GB = 1024 MiB.

## Amazon EC2 Task CPU and Memory

For tasks hosted on Amazon EC2, supported task CPU values are between 0.25 vCPUs and 192 vCPUs.

## CPU Control Mechanism Differences

The CPU control mechanism differs between EC2 and Fargate:

- **For tasks hosted on Amazon EC2:** Amazon ECS uses the CPU period and the CPU quota to control the task size CPU hard limits. When you specify the vCPU in your task definition, Amazon ECS translates the value to the CPU period and CPU quota settings that apply to the `cgroup`.

- **For tasks hosted on Fargate:** Amazon ECS uses CPU shares to control CPU allocation. The CPU quota and period values are not used for CPU limiting in Fargate tasks.

## CPU Quota and Period Details

For Amazon EC2 tasks, the CPU quota controls the amount of CPU time granted to a `cgroup` during a given CPU period. Both settings are expressed in terms of microseconds. When the CPU quota equals the CPU period, a `cgroup` can execute up to 100% on one vCPU (or any other fraction that totals to 100% for multiple vCPUs). The CPU quota has a maximum of 1000000us and the CPU period has a minimum of 1ms. You can use these values to set the limits for your CPU count. When you change the CPU period without changing the CPU quota, you have different effective limits than what you've specified in your task definition.

The 100ms period allows for vCPUs ranging from 0.125 to 10.

> **Important:** Task-level CPU and memory parameters are ignored for Windows containers.
