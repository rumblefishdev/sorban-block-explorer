import { Module } from '@nestjs/common';
import { ConfigModule } from '@nestjs/config';
import { APP_FILTER } from '@nestjs/core';
import { DatabaseModule } from './database/database.module.js';
import { GlobalExceptionFilter } from './filters/global-exception.filter.js';
import { NetworkModule } from './modules/network/network.module.js';
import { TransactionsModule } from './modules/transactions/transactions.module.js';
import { LedgersModule } from './modules/ledgers/ledgers.module.js';
import { AccountsModule } from './modules/accounts/accounts.module.js';
import { TokensModule } from './modules/tokens/tokens.module.js';
import { ContractsModule } from './modules/contracts/contracts.module.js';
import { NftsModule } from './modules/nfts/nfts.module.js';
import { LiquidityPoolsModule } from './modules/liquidity-pools/liquidity-pools.module.js';
import { SearchModule } from './modules/search/search.module.js';
import { HealthController } from './health.controller.js';

@Module({
  imports: [
    ConfigModule.forRoot({
      isGlobal: true,
      cache: true,
    }),
    DatabaseModule,
    NetworkModule,
    TransactionsModule,
    LedgersModule,
    AccountsModule,
    TokensModule,
    ContractsModule,
    NftsModule,
    LiquidityPoolsModule,
    SearchModule,
  ],
  controllers: [HealthController],
  providers: [
    {
      provide: APP_FILTER,
      useClass: GlobalExceptionFilter,
    },
  ],
})
export class AppModule {}
