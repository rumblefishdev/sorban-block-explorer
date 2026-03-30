import 'reflect-metadata';
import { NestFactory } from '@nestjs/core';
import type { Handler } from 'aws-lambda';
import { AppModule } from './app.module.js';

let server: Handler | undefined;

async function bootstrap(): Promise<Handler> {
  const { configure } = await import('@codegenie/serverless-express');
  const app = await NestFactory.create(AppModule, {
    logger: ['error', 'warn', 'log'],
  });
  app.setGlobalPrefix('v1');
  await app.init();
  const expressApp = app.getHttpAdapter().getInstance();
  return configure({ app: expressApp });
}

export const handler: Handler = async (event, context, callback) => {
  server = server ?? (await bootstrap());
  return server(event, context, callback);
};
