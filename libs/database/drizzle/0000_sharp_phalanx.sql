CREATE TABLE "ledgers" (
	"sequence" bigint PRIMARY KEY NOT NULL,
	"hash" varchar(64) NOT NULL,
	"closed_at" timestamp with time zone NOT NULL,
	"protocol_version" integer NOT NULL,
	"transaction_count" integer NOT NULL,
	"base_fee" bigint NOT NULL,
	CONSTRAINT "ledgers_hash_unique" UNIQUE("hash")
);
--> statement-breakpoint
CREATE TABLE "transactions" (
	"id" bigserial PRIMARY KEY NOT NULL,
	"hash" varchar(64) NOT NULL,
	"ledger_sequence" bigint NOT NULL,
	"source_account" varchar(56) NOT NULL,
	"fee_charged" bigint NOT NULL,
	"successful" boolean NOT NULL,
	"result_code" varchar(50),
	"envelope_xdr" text NOT NULL,
	"result_xdr" text NOT NULL,
	"result_meta_xdr" text,
	"memo_type" varchar(20),
	"memo" text,
	"created_at" timestamp with time zone NOT NULL,
	"parse_error" boolean DEFAULT false,
	"operation_tree" jsonb,
	CONSTRAINT "transactions_hash_unique" UNIQUE("hash")
);
--> statement-breakpoint
ALTER TABLE "transactions" ADD CONSTRAINT "transactions_ledger_sequence_ledgers_sequence_fk" FOREIGN KEY ("ledger_sequence") REFERENCES "public"."ledgers"("sequence") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "idx_closed_at" ON "ledgers" USING btree ("closed_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "idx_source" ON "transactions" USING btree ("source_account","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "idx_ledger" ON "transactions" USING btree ("ledger_sequence");