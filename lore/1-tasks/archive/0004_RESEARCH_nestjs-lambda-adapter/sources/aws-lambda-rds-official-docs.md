---
url: 'https://docs.aws.amazon.com/lambda/latest/dg/services-rds.html'
title: 'Using AWS Lambda with Amazon RDS'
date: '2024-01-01'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# Using AWS Lambda with Amazon RDS

## Overview

You can connect a Lambda function to an Amazon Relational Database Service (Amazon RDS) database directly and through an Amazon RDS Proxy. Direct connections are useful in simple scenarios; proxies are recommended for production. A database proxy manages a pool of shared database connections which enables your function to reach high concurrency levels without exhausting database connections.

AWS recommends using Amazon RDS Proxy for Lambda functions that make frequent short database connections, or open and close large numbers of database connections.

## Quick Connection via Console

1. Open the [Functions page](https://console.aws.amazon.com/lambda/home#/functions) of the Lambda console
2. Select the function you want to connect a database to
3. On the **Configuration** tab, select **RDS databases**
4. Choose **Connect to RDS database**

After connecting, create a proxy by choosing **Add proxy**.

## Configuration Requirements

- Lambda function must be in the same Amazon VPC as the database
- Supported engines: MySQL, MariaDB, PostgreSQL, Microsoft SQL Server, Aurora (MySQL or PostgreSQL)
- A Secrets Manager secret is required for database authentication
- An IAM role must grant permission to use the secret
- IAM trust policy must allow Amazon RDS to assume the role

## Required IAM Permissions

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "ec2:CreateSecurityGroup",
        "ec2:DescribeSecurityGroups",
        "ec2:DescribeSubnets",
        "ec2:DescribeVpcs",
        "ec2:AuthorizeSecurityGroupIngress",
        "ec2:AuthorizeSecurityGroupEgress",
        "ec2:RevokeSecurityGroupEgress",
        "ec2:CreateNetworkInterface",
        "ec2:DeleteNetworkInterface",
        "ec2:DescribeNetworkInterfaces"
      ],
      "Resource": "*"
    },
    {
      "Effect": "Allow",
      "Action": [
        "rds-db:connect",
        "rds:CreateDBProxy",
        "rds:CreateDBInstance",
        "rds:CreateDBSubnetGroup",
        "rds:DescribeDBClusters",
        "rds:DescribeDBInstances",
        "rds:DescribeDBSubnetGroups",
        "rds:DescribeDBProxies",
        "rds:DescribeDBProxyTargets",
        "rds:DescribeDBProxyTargetGroups",
        "rds:RegisterDBProxyTargets",
        "rds:ModifyDBInstance",
        "rds:ModifyDBProxy"
      ],
      "Resource": "*"
    },
    {
      "Effect": "Allow",
      "Action": [
        "lambda:CreateFunction",
        "lambda:ListFunctions",
        "lambda:UpdateFunctionConfiguration"
      ],
      "Resource": "*"
    },
    {
      "Effect": "Allow",
      "Action": ["iam:AttachRolePolicy", "iam:CreateRole", "iam:CreatePolicy"],
      "Resource": "*"
    },
    {
      "Effect": "Allow",
      "Action": [
        "secretsmanager:GetResourcePolicy",
        "secretsmanager:GetSecretValue",
        "secretsmanager:DescribeSecret",
        "secretsmanager:ListSecretVersionIds",
        "secretsmanager:CreateSecret"
      ],
      "Resource": "*"
    }
  ]
}
```

## SSL/TLS Certificate Handling

### .zip File Archives

- **Node.js 18 and earlier**: Lambda automatically includes CA certificates and RDS certificates
- **Node.js 20 and later**: Lambda no longer loads additional CA certificates by default. Set the `NODE_EXTRA_CA_CERTS` environment variable to `/var/runtime/ca-cert.pem`

### Container Images

AWS base images include only CA certificates. Include the appropriate certificates in your container image:

```dockerfile
RUN curl https://truststore.pki.rds.amazonaws.com/us-east-1/us-east-1-bundle.pem -o /us-east-1-bundle.pem
```

### Node.js Connection Config (with SSL)

```javascript
import { readFileSync } from 'fs';

let connectionConfig = {
  host: process.env.ProxyHostName,
  user: process.env.DBUserName,
  password: token,
  database: process.env.DBName,
  ssl: {
    ca: readFileSync('/us-east-1-bundle.pem'),
  },
};
```

## JavaScript Example: IAM Auth Token + MySQL

```javascript
import { Signer } from '@aws-sdk/rds-signer';
import mysql from 'mysql2/promise';

async function createAuthToken() {
  const dbinfo = {
    hostname: process.env.ProxyHostName,
    port: process.env.Port,
    username: process.env.DBUserName,
    region: process.env.AWS_REGION,
  };

  const signer = new Signer(dbinfo);
  const token = await signer.getAuthToken();
  return token;
}

async function dbOps() {
  const token = await createAuthToken();
  let connectionConfig = {
    host: process.env.ProxyHostName,
    user: process.env.DBUserName,
    password: token,
    database: process.env.DBName,
    ssl: 'Amazon RDS',
  };
  const conn = await mysql.createConnection(connectionConfig);
  const [res] = await conn.execute('select ?+? as sum', [3, 2]);
  return res;
}

export const handler = async (event) => {
  const result = await dbOps();
  return {
    statusCode: 200,
    body: JSON.stringify('The selected sum is: ' + result[0].sum),
  };
};
```

## TypeScript Example: IAM Auth Token

```typescript
import { Signer } from '@aws-sdk/rds-signer';
import mysql from 'mysql2/promise';

const proxy_host_name = process.env.PROXY_HOST_NAME!;
const port = parseInt(process.env.PORT!);
const db_name = process.env.DB_NAME!;
const db_user_name = process.env.DB_USER_NAME!;
const aws_region = process.env.AWS_REGION!;

async function createAuthToken(): Promise<string> {
  const signer = new Signer({
    hostname: proxy_host_name,
    port: port,
    region: aws_region,
    username: db_user_name,
  });

  const token = await signer.getAuthToken();
  return token;
}

async function dbOps(): Promise<mysql.QueryResult | undefined> {
  try {
    const token = await createAuthToken();
    const conn = await mysql.createConnection({
      host: proxy_host_name,
      user: db_user_name,
      password: token,
      database: db_name,
      ssl: 'Amazon RDS',
    });
    const [rows, fields] = await conn.execute('SELECT ? + ? AS sum', [3, 2]);
    return rows;
  } catch (err) {
    console.log(err);
  }
}

export const lambdaHandler = async (
  event: any
): Promise<{ statusCode: number; body: string }> => {
  const result = await dbOps();

  if (result == undefined)
    return {
      statusCode: 500,
      body: JSON.stringify(`Error with connection to DB host`),
    };

  return {
    statusCode: 200,
    body: JSON.stringify(`The selected sum is: ${result[0].sum}`),
  };
};
```

## Processing Event Notifications from Amazon RDS

Lambda can process event notifications from an Amazon RDS database. Amazon RDS sends notifications to an SNS topic, which can invoke a Lambda function. Amazon SNS wraps the RDS message in its own event document.

### Example Amazon RDS Message in an SNS Event

```json
{
  "Records": [
    {
      "EventVersion": "1.0",
      "EventSubscriptionArn": "arn:aws:sns:us-east-2:123456789012:rds-lambda:21be56ed-...",
      "EventSource": "aws:sns",
      "Sns": {
        "MessageId": "95df01b4-ee98-5cb9-9903-4c221d41eb5e",
        "Message": "{\"Event Source\":\"db-instance\",\"Event Time\":\"2023-01-02 12:45:06.000\",\"Source ID\":\"dbinstanceid\",\"Event Message\":\"Finished DB Instance backup\"}",
        "Type": "Notification",
        "TopicArn": "arn:aws:sns:us-east-2:123456789012:sns-lambda"
      }
    }
  ]
}
```

## Further Reading

- [Using a Lambda function to access an Amazon RDS database](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/rds-lambda-tutorial.html) — Covers reading SQS records and writing to RDS through RDS Proxy
