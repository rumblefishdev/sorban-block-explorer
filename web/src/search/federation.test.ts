import { afterEach, describe, expect, it, vi } from 'vitest';

import { fetchReply, stubFetch } from '../test-utils.js';

import {
  federatedDomain,
  resolveFederated,
  resolveFederatedName,
} from './federation.js';

const ACCOUNT = 'GC526FUILJ6NLFXKCOOGTMDXNRW7MYSEK2UNRJV5FYWOGYDE4LOKXFEM';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('federatedDomain', () => {
  it('accepts a federated address and lowercases the domain', () => {
    expect(federatedDomain('karol*LOBSTR.co')).toBe('lobstr.co');
    expect(federatedDomain('  a*sub.example.org  ')).toBe('sub.example.org');
  });

  // The name shapes SEP-2 lists verbatim as valid: an email address and an
  // E.164 phone number both carry characters a naive `\w+` would drop.
  it.each([
    ['a plain name', 'jed*stellar.org', 'stellar.org'],
    ['an email as the name', 'bob@gmail.com*stellar.org', 'stellar.org'],
    ['an E.164 phone number', '+14155550100*stellar.org', 'stellar.org'],
    ['an internationalized domain', 'karol*münchen.de', 'münchen.de'],
    ['a punycode domain', 'karol*xn--mnchen-3ya.de', 'xn--mnchen-3ya.de'],
  ])('accepts %s', (_label, input, domain) => {
    expect(federatedDomain(input)).toBe(domain);
  });

  // The classifier is the gate on spending a network round-trip, so every
  // near-miss below must stay a miss.
  it.each([
    ['a plain account id', ACCOUNT],
    ['no star', 'karol.lobstr.co'],
    ['two stars', 'a*b*c.co'],
    ['no domain dot', 'karol*localhost'],
    ['hyphenated but dotless domain', 'not*a-domain'],
    // SEP-2 excludes `>` from the username outright.
    ['a `>` in the name', 'bo>b*stellar.org'],
    // Stricter than SEP-2 on purpose — see the FEDERATED doc comment.
    ['a single-label domain', 'karol*localhost'],
    ['numeric tld', 'karol*example.12'],
    ['whitespace inside', 'kar ol*lobstr.co'],
    ['empty name', '*lobstr.co'],
    ['empty', ''],
  ])('rejects %s', (_label, input) => {
    expect(federatedDomain(input)).toBeNull();
  });
});

describe('resolveFederated', () => {
  it('resolves through stellar.toml to the account id', async () => {
    const fetchMock = stubFetch({
      'https://lobstr.co/.well-known/stellar.toml': fetchReply(
        'NETWORK_PASSPHRASE = "Public"\nFEDERATION_SERVER="https://lobstr.co/federation/"\n'
      ),
      'https://lobstr.co/federation/': fetchReply(
        JSON.stringify({
          stellar_address: 'karol*lobstr.co',
          account_id: ACCOUNT,
        })
      ),
    });

    await expect(
      resolveFederated('karol*lobstr.co', 'lobstr.co')
    ).resolves.toEqual({
      kind: 'resolved',
      accountId: ACCOUNT,
    });

    const federationCall = fetchMock.mock.calls[1]?.[0] as string;
    expect(federationCall).toContain('type=name');
    expect(federationCall).toContain('q=karol*lobstr.co');
  });

  // The domain owner learns the viewer's IP by virtue of being fetched. It
  // must not also learn which page they came from, and none of our cookies
  // may travel with the request.
  it('sends no referrer and no credentials', async () => {
    const fetchMock = stubFetch({
      'https://lobstr.co/.well-known/stellar.toml': fetchReply(
        'FEDERATION_SERVER="https://lobstr.co/federation/"\n'
      ),
      'https://lobstr.co/federation/': fetchReply(
        JSON.stringify({ account_id: ACCOUNT })
      ),
    });

    await resolveFederated('karol*lobstr.co', 'lobstr.co');

    for (const call of fetchMock.mock.calls) {
      const init = (call as unknown as [string, RequestInit])[1];
      expect(init.referrerPolicy).toBe('no-referrer');
      expect(init.credentials).toBe('omit');
    }
  });

  it('fails explicitly when the domain serves no stellar.toml', async () => {
    stubFetch({});
    const r = await resolveFederated('karol*lobstr.co', 'lobstr.co');
    expect(r.kind).toBe('failed');
    expect(r.kind === 'failed' && r.reason).toContain('stellar.toml');
  });

  it('fails explicitly when the toml declares no federation server', async () => {
    stubFetch({
      'https://lobstr.co/.well-known/stellar.toml': fetchReply(
        'VERSION = "2.0.0"\n'
      ),
    });
    const r = await resolveFederated('karol*lobstr.co', 'lobstr.co');
    expect(r.kind).toBe('failed');
    expect(r.kind === 'failed' && r.reason).toContain('no federation server');
  });

  // A plaintext answer decides which account the user is sent to.
  it('refuses a non-HTTPS federation server without calling it', async () => {
    const fetchMock = stubFetch({
      'https://lobstr.co/.well-known/stellar.toml': fetchReply(
        'FEDERATION_SERVER="http://lobstr.co/federation/"\n'
      ),
    });
    const r = await resolveFederated('karol*lobstr.co', 'lobstr.co');
    expect(r.kind).toBe('failed');
    expect(r.kind === 'failed' && r.reason).toContain('non-HTTPS');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  // The server is run by the domain owner; its answer becomes a route.
  it.each([
    ['a non-account string', JSON.stringify({ account_id: 'not-a-key' })],
    ['a contract id', JSON.stringify({ account_id: `C${ACCOUNT.slice(1)}` })],
    ['no account_id', JSON.stringify({ detail: 'not found' })],
    ['unparseable json', '<html>404</html>'],
  ])('rejects %s from the federation server', async (_label, body) => {
    stubFetch({
      'https://lobstr.co/.well-known/stellar.toml': fetchReply(
        'FEDERATION_SERVER="https://lobstr.co/federation/"\n'
      ),
      'https://lobstr.co/federation/': fetchReply(body),
    });
    const r = await resolveFederated('karol*lobstr.co', 'lobstr.co');
    expect(r.kind).toBe('failed');
  });

  it('fails explicitly when the federation server 404s the name', async () => {
    stubFetch({
      'https://lobstr.co/.well-known/stellar.toml': fetchReply(
        'FEDERATION_SERVER="https://lobstr.co/federation/"\n'
      ),
      'https://lobstr.co/federation/': fetchReply(
        '{"detail":"not found"}',
        false
      ),
    });
    const r = await resolveFederated('nobody*lobstr.co', 'lobstr.co');
    expect(r.kind).toBe('failed');
    expect(r.kind === 'failed' && r.reason).toContain('did not resolve');
  });
});

describe('resolveFederatedName (reverse, type=id)', () => {
  const TOML = 'https://lobstr.co/.well-known/stellar.toml';
  const SERVER = 'https://lobstr.co/federation/';
  const withServer = {
    [TOML]: fetchReply('FEDERATION_SERVER="https://lobstr.co/federation/"\n'),
  };

  it('returns the address the domain claims for the account', async () => {
    const fetchMock = stubFetch({
      ...withServer,
      [SERVER]: fetchReply(
        JSON.stringify({ stellar_address: 'karol*lobstr.co' })
      ),
    });

    await expect(resolveFederatedName(ACCOUNT, 'lobstr.co')).resolves.toBe(
      'karol*lobstr.co'
    );
    expect(fetchMock.mock.calls[1]?.[0] as string).toContain('type=id');
  });

  // The account named this domain; the domain must name the account back
  // inside its OWN namespace, or it is claiming it into someone else's.
  it('rejects an address that does not live at the account home domain', async () => {
    stubFetch({
      ...withServer,
      [SERVER]: fetchReply(
        JSON.stringify({ stellar_address: 'karol*evil.example' })
      ),
    });

    await expect(
      resolveFederatedName(ACCOUNT, 'lobstr.co')
    ).resolves.toBeNull();
  });

  it('is silent when the account has no registered name', async () => {
    stubFetch({
      ...withServer,
      [SERVER]: fetchReply(JSON.stringify({ detail: 'not found' })),
    });
    await expect(
      resolveFederatedName(ACCOUNT, 'lobstr.co')
    ).resolves.toBeNull();
  });

  // `home_domain` is free text on the ledger. Measured in production: 7484
  // accounts carry one with no dot in it, so this gate is the difference
  // between a quiet account page and one that dials a host that cannot exist.
  it.each([
    ['empty', ''],
    ['a label with no dot', 'Bankless'],
    ['a country name', 'Indonesia'],
    ['a host with a port', 'localhost:4000'],
    ['a bare digit', '1'],
    ['a space', ' '],
  ])('makes no request for %s', async (_label, homeDomain) => {
    const fetchMock = stubFetch({});
    await expect(resolveFederatedName(ACCOUNT, homeDomain)).resolves.toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
