---
url: 'https://docs.aws.amazon.com/vpc/latest/privatelink/aws-services-privatelink-support.html'
title: 'AWS services that integrate with AWS PrivateLink'
fetched_date: 2026-03-25
task_id: '0001'
image_count: 0
---

# AWS services that integrate with AWS PrivateLink

The following AWS services integrate with AWS PrivateLink. You can create a VPC endpoint to connect to these services privately, as if they were running in your own VPC.

Choose the link in the **AWS service** column to see the documentation for services that integrate with AWS PrivateLink. The **Service name** column contains the service name that you specify when you create the interface VPC endpoint, or it indicates that the service manages the endpoint.

## Services relevant to Galexie / ECS / ECR workloads

| AWS service                                                                                                               | Service name                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [Amazon ECR](https://docs.aws.amazon.com/AmazonECR/latest/userguide/vpc-endpoints.html)                                   | `com.amazonaws.region.ecr.api`<br>`com.amazonaws.region.ecr.dkr`                                                                                                                                                               |
| [Amazon S3](https://docs.aws.amazon.com/AmazonS3/latest/userguide/privatelink-interface-endpoints.html)                   | `com.amazonaws.region.s3`<br>`com.amazonaws.region.s3tables`                                                                                                                                                                   |
| [Amazon CloudWatch Logs](https://docs.aws.amazon.com/AmazonCloudWatch/latest/logs/cloudwatch-logs-and-interface-VPC.html) | `com.amazonaws.region.logs`                                                                                                                                                                                                    |
| [AWS Systems Manager](https://docs.aws.amazon.com/systems-manager/latest/userguide/setup-create-vpc.html)                 | `com.amazonaws.region.ssm`<br>`com.amazonaws.region.ec2messages`<br>`com.amazonaws.region.ssmmessages`<br>`com.amazonaws.region.ssm-contacts`<br>`com.amazonaws.region.ssm-incidents`<br>`com.amazonaws.region.ssm-quicksetup` |
| [Amazon ECS](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/vpc-endpoints.html)                              | `com.amazonaws.region.ecs`<br>`com.amazonaws.region.ecs-agent`<br>`com.amazonaws.region.ecs-telemetry`                                                                                                                         |

## Full service table

| AWS service                                                                                                                           | Service name                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [AWS Account Management](https://docs.aws.amazon.com/accounts/latest/reference/security-privatelink.html)                             | `com.amazonaws.region.account`                                                                                                                                                                                                                                                                                                                                 |
| [Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/apigateway-private-apis.html)                       | `com.amazonaws.region.execute-api`<br>`com.amazonaws.region.apigateway`                                                                                                                                                                                                                                                                                        |
| [Amazon CloudWatch](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/cloudwatch-and-interface-VPC.html)                 | `com.amazonaws.region.monitoring`<br>`com.amazonaws.region.application-signals`<br>`com.amazonaws.region.applicationinsights`<br>`com.amazonaws.region.internetmonitor`<br>`com.amazonaws.region.networkflowmonitor`<br>`com.amazonaws.region.networkmonitor`<br>`com.amazonaws.region.rum`<br>`com.amazonaws.region.synthetics`<br>`com.amazonaws.region.oam` |
| [Amazon CloudWatch Logs](https://docs.aws.amazon.com/AmazonCloudWatch/latest/logs/cloudwatch-logs-and-interface-VPC.html)             | `com.amazonaws.region.logs`                                                                                                                                                                                                                                                                                                                                    |
| [Amazon DynamoDB](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/privatelink-interface-endpoints.html)              | `com.amazonaws.region.dynamodb`<br>`com.amazonaws.region.dynamodb-fips`<br>`com.amazonaws.region.dynamodb-streams`                                                                                                                                                                                                                                             |
| [Amazon EC2](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/interface-vpc-endpoints.html)                                        | `com.amazonaws.region.ec2`<br>`com.amazonaws.region.ec2-fips`                                                                                                                                                                                                                                                                                                  |
| [Amazon ECR](https://docs.aws.amazon.com/AmazonECR/latest/userguide/vpc-endpoints.html)                                               | `com.amazonaws.region.ecr.api`<br>`com.amazonaws.region.ecr.dkr`                                                                                                                                                                                                                                                                                               |
| [Amazon ECS](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/vpc-endpoints.html)                                          | `com.amazonaws.region.ecs`<br>`com.amazonaws.region.ecs-agent`<br>`com.amazonaws.region.ecs-telemetry`                                                                                                                                                                                                                                                         |
| [Amazon EKS](https://docs.aws.amazon.com/eks/latest/userguide/vpc-interface-endpoints.html)                                           | `com.amazonaws.region.eks`<br>`com.amazonaws.region.eks-auth`<br>`com.amazonaws.region.eks-fips`<br>`com.amazonaws.region.eks-proxy`                                                                                                                                                                                                                           |
| [AWS Key Management Service](https://docs.aws.amazon.com/kms/latest/developerguide/kms-vpc-endpoint.html)                             | `com.amazonaws.region.kms`<br>`com.amazonaws.region.kms-fips`                                                                                                                                                                                                                                                                                                  |
| [AWS Lambda](https://docs.aws.amazon.com/lambda/latest/dg/configuration-vpc-endpoints.html)                                           | `com.amazonaws.region.lambda`                                                                                                                                                                                                                                                                                                                                  |
| [Amazon S3](https://docs.aws.amazon.com/AmazonS3/latest/userguide/privatelink-interface-endpoints.html)                               | `com.amazonaws.region.s3`<br>`com.amazonaws.region.s3tables`                                                                                                                                                                                                                                                                                                   |
| [Amazon S3 Multi-Region Access Points](https://docs.aws.amazon.com/AmazonS3/latest/userguide/MultiRegionAccessPointsPrivateLink.html) | `com.amazonaws.s3-global.accesspoint`                                                                                                                                                                                                                                                                                                                          |
| [AWS Secrets Manager](https://docs.aws.amazon.com/secretsmanager/latest/userguide/vpc-endpoint-overview.html)                         | `com.amazonaws.region.secretsmanager`                                                                                                                                                                                                                                                                                                                          |
| [AWS Security Token Service](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_interface_vpc_endpoints.html)                 | `com.amazonaws.region.sts`<br>`com.amazonaws.region.sts-fips`                                                                                                                                                                                                                                                                                                  |
| [AWS Systems Manager](https://docs.aws.amazon.com/systems-manager/latest/userguide/setup-create-vpc.html)                             | `com.amazonaws.region.ssm`<br>`com.amazonaws.region.ec2messages`<br>`com.amazonaws.region.ssm-contacts`<br>`com.amazonaws.region.ssm-incidents`<br>`com.amazonaws.region.ssm-incidents-fips`<br>`com.amazonaws.region.ssm-quicksetup`<br>`com.amazonaws.region.ssmmessages`                                                                                    |

## View available AWS service names

You can use the [describe-vpc-endpoint-services](https://docs.aws.amazon.com/cli/latest/reference/ec2/describe-vpc-endpoint-services.html) command to view the service names that support VPC endpoints.

```bash
aws ec2 describe-vpc-endpoint-services \
  --filters Name=service-type,Values=Interface Name=owner,Values=amazon \
  --region us-east-1 \
  --query ServiceNames
```

## View endpoint policy support

```bash
aws ec2 describe-vpc-endpoint-services \
  --service-name "com.amazonaws.us-east-1.s3" \
  --region us-east-1 \
  --query ServiceDetails[*].VpcEndpointPolicySupported \
  --output text
```
