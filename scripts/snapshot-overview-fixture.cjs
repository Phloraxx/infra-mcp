#!/usr/bin/env node
// Regenerate scripts/fixtures/overview-snapshot.json from a gateway /api/overview.
//
// The browser gate runs a second pass against this file, so a field the gateway
// adds, renames or drops fails the gate instead of shipping. The output is
// redacted: device ids, device names, boot ids, request ids and every UUID become
// synthetic values, and every *_unix_ms timestamp is rebased onto a fixed epoch,
// so no real device identity, wall-clock time or operational topology enters the
// repository.
//
// Live capture (preferred):
//   INFRA_SNAPSHOT_URL=https://staging.example \
//   INFRA_SNAPSHOT_TOKEN=<admin bearer> \
//   node scripts/snapshot-overview-fixture.cjs
//
// Offline capture, derived from the gateway response types:
//   node scripts/snapshot-overview-fixture.cjs --from-schema
//
// Only ever point this at a gateway you own. Never commit an unredacted response.
'use strict';
const fs = require('node:fs');
const path = require('node:path');

// Fixed epoch the fixture is rebased onto. The gate shifts the timestamps back
// onto the current clock before serving them, so relative ages stay realistic
// while the committed file carries no real capture time.
const FIXTURE_EPOCH_MS = 1767225600000; // 2026-01-01T00:00:00Z
const OUTPUT = path.resolve(__dirname, 'fixtures/overview-snapshot.json');
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// The exact response shape the dashboard consumes. These mirror McpDeviceView
// (crates/gateway/src/main.rs), DashboardSession (crates/gateway/src/dashboard_http.rs)
// and AuditRecord (crates/gateway/src/store.rs). A response that does not match
// one of these lists is schema drift and must be resolved, not papered over.
const DEVICE_FIELDS = [
  'device_id',
  'device_name',
  'agent_version',
  'platform',
  'arch',
  'capabilities',
  'online',
  'revoked',
  'last_seen_unix_ms',
  'connection_state',
  'last_disconnect_reason',
  'last_disconnect_at_unix_ms',
  'connection_generation',
  'active_jobs',
  'active_terminals',
  'active_transfers',
];
const SESSION_FIELDS = [
  'id',
  'kind',
  'status',
  'boot_id',
  'started_at_unix_ms',
  'device_id',
  'device_name',
  'device_online',
];
const ACTIVITY_FIELDS = [
  'id',
  'at_unix_ms',
  'device_id',
  'device_name',
  'action',
  'outcome',
  'request_id',
];

function fail(message) {
  console.error(`snapshot-overview-fixture: ${message}`);
  process.exit(1);
}

function assertShape(kind, entries, fields) {
  if (!Array.isArray(entries) || entries.length === 0) {
    fail(`the gateway returned no ${kind} entries; capture a gateway with at least one`);
  }
  const expected = [...fields].sort();
  for (const entry of entries) {
    const actual = Object.keys(entry).sort();
    if (actual.join(',') !== expected.join(',')) {
      const added = actual.filter(key => !expected.includes(key));
      const missing = expected.filter(key => !actual.includes(key));
      fail(
        `${kind} shape drifted from the dashboard contract` +
          (added.length ? `; new fields: ${added.join(', ')}` : '') +
          (missing.length ? `; missing fields: ${missing.join(', ')}` : '')
      );
    }
  }
}

function makeRedactor() {
  const maps = {device: new Map(), name: new Map(), boot: new Map(), request: new Map(), uuid: new Map()};
  const assign = (kind, value, format) => {
    if (typeof value !== 'string' || !value) return value;
    if (!maps[kind].has(value)) maps[kind].set(value, format(maps[kind].size + 1));
    return maps[kind].get(value);
  };
  return {
    deviceId: value => assign('device', value, n => `device-${n}`),
    deviceName: value => assign('name', value, n => `Device ${n}`),
    bootId: value => assign('boot', value, n => `boot-${n}`),
    requestId: value => assign('request', value, n => `request-${n}`),
    uuid: value =>
      assign('uuid', value, n => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`),
  };
}

function redact(overview, capturedAtMs) {
  const redactor = makeRedactor();
  const shift = FIXTURE_EPOCH_MS - capturedAtMs;
  const rebase = value => (typeof value === 'number' ? value + shift : value);
  const optional = (value, map) => (value === null || value === undefined ? value : map(value));
  return {
    _meta: {
      generated_by: 'scripts/snapshot-overview-fixture.cjs',
      redacted: true,
      epoch_unix_ms: FIXTURE_EPOCH_MS,
    },
    devices: overview.devices.map(device => ({
      device_id: redactor.deviceId(device.device_id),
      device_name: redactor.deviceName(device.device_name),
      agent_version: 'redacted',
      platform: device.platform,
      arch: device.arch,
      capabilities: device.capabilities,
      online: device.online,
      revoked: device.revoked,
      last_seen_unix_ms: rebase(device.last_seen_unix_ms),
      connection_state: device.connection_state,
      last_disconnect_reason: device.last_disconnect_reason,
      last_disconnect_at_unix_ms: rebase(device.last_disconnect_at_unix_ms),
      connection_generation: device.connection_generation,
      active_jobs: device.active_jobs,
      active_terminals: device.active_terminals,
      active_transfers: device.active_transfers,
    })),
    sessions: overview.sessions.map(session => ({
      id: redactor.uuid(session.id),
      kind: session.kind,
      status: session.status,
      boot_id: redactor.bootId(session.boot_id),
      started_at_unix_ms: rebase(session.started_at_unix_ms),
      device_id: redactor.deviceId(session.device_id),
      device_name: redactor.deviceName(session.device_name),
      device_online: session.device_online,
    })),
    activity: overview.activity.map(record => ({
      id: record.id,
      at_unix_ms: rebase(record.at_unix_ms),
      device_id: optional(record.device_id, redactor.deviceId),
      device_name: optional(record.device_name, redactor.deviceName),
      action: record.action,
      outcome: record.outcome,
      request_id: optional(record.request_id, redactor.requestId),
    })),
  };
}

// Offline capture. Every field the gateway emits is present, including the
// optional ones the hand-written QA fixture omits, so the second gate pass
// exercises the render paths that only a real response reaches.
function schemaOverview() {
  const now = FIXTURE_EPOCH_MS;
  return {
    devices: [
      {
        device_id: 'device-1',
        device_name: 'Device 1',
        agent_version: 'redacted',
        platform: 'linux',
        arch: 'aarch64',
        capabilities: ['exec', 'files', 'files.list_page', 'terminal', 'jobs', 'transfer', 'connection.readiness_v1'],
        online: true,
        revoked: false,
        last_seen_unix_ms: now - 4000,
        connection_state: 'online',
        last_disconnect_reason: null,
        last_disconnect_at_unix_ms: null,
        connection_generation: 7,
        active_jobs: 1,
        active_terminals: 1,
        active_transfers: 0,
      },
      {
        device_id: 'device-2',
        device_name: 'Device 2',
        agent_version: 'redacted',
        platform: 'darwin',
        arch: 'aarch64',
        capabilities: ['exec', 'files', 'terminal', 'jobs', 'transfer'],
        online: false,
        revoked: false,
        last_seen_unix_ms: now - 540000,
        connection_state: 'reconnecting',
        last_disconnect_reason: 'heartbeat_timeout',
        last_disconnect_at_unix_ms: now - 600000,
        connection_generation: null,
        active_jobs: 0,
        active_terminals: 0,
        active_transfers: 0,
      },
      {
        device_id: 'device-3',
        device_name: 'Device 3',
        agent_version: 'redacted',
        platform: 'windows',
        arch: 'x86_64',
        capabilities: ['exec', 'files', 'terminal'],
        online: false,
        revoked: true,
        last_seen_unix_ms: now - 86400000,
        connection_state: 'revoked',
        last_disconnect_reason: 'socket_closed',
        last_disconnect_at_unix_ms: now - 86400000,
        connection_generation: null,
        active_jobs: 0,
        active_terminals: 0,
        active_transfers: 0,
      },
    ],
    sessions: [
      {
        id: '00000000-0000-4000-8000-000000000001',
        kind: 'terminal',
        status: 'running',
        boot_id: 'boot-1',
        started_at_unix_ms: now - 120000,
        device_id: 'device-1',
        device_name: 'Device 1',
        device_online: true,
      },
      {
        id: '00000000-0000-4000-8000-000000000002',
        kind: 'job',
        status: 'running',
        boot_id: 'boot-1',
        started_at_unix_ms: now - 300000,
        device_id: 'device-1',
        device_name: 'Device 1',
        device_online: true,
      },
      {
        id: '00000000-0000-4000-8000-000000000003',
        kind: 'transfer',
        status: 'ready',
        boot_id: 'boot-1',
        started_at_unix_ms: now - 60000,
        device_id: 'device-1',
        device_name: 'Device 1',
        device_online: true,
      },
    ],
    activity: [
      {
        id: 1,
        at_unix_ms: now - 5000,
        device_id: 'device-1',
        device_name: 'Device 1',
        action: 'device.connect',
        outcome: 'ok',
        request_id: null,
      },
      {
        id: 2,
        at_unix_ms: now - 30000,
        device_id: 'device-1',
        device_name: 'Device 1',
        action: 'exec',
        outcome: 'ok',
        request_id: 'request-1',
      },
      {
        id: 3,
        at_unix_ms: now - 600000,
        device_id: 'device-2',
        device_name: 'Device 2',
        action: 'device.disconnect',
        outcome: 'ok',
        request_id: null,
      },
    ],
  };
}

async function capture() {
  const base = process.env.INFRA_SNAPSHOT_URL;
  const token = process.env.INFRA_SNAPSHOT_TOKEN;
  if (!base) return null;
  if (!token) fail('INFRA_SNAPSHOT_TOKEN is required when INFRA_SNAPSHOT_URL is set');
  const url = `${base.replace(/\/+$/, '')}/api/overview`;
  const response = await fetch(url, {headers: {Authorization: `Bearer ${token}`}});
  if (!response.ok) fail(`${url} returned HTTP ${response.status}`);
  const overview = await response.json();
  for (const [kind, entries, fields] of [
    ['device', overview.devices, DEVICE_FIELDS],
    ['session', overview.sessions, SESSION_FIELDS],
    ['activity', overview.activity, ACTIVITY_FIELDS],
  ]) {
    assertShape(kind, entries, fields);
  }
  return {overview, source: 'live-gateway'};
}

(async () => {
  const fromSchema = process.argv.includes('--from-schema');
  const captured = fromSchema ? null : await capture();
  const overview = captured ? captured.overview : schemaOverview();
  const source = captured ? captured.source : 'gateway-response-schema';
  if (!captured) {
    console.log(
      'snapshot-overview-fixture: no INFRA_SNAPSHOT_URL, writing the fixture from the gateway response schema'
    );
  }
  // The schema fixture is already expressed against the fixed epoch.
  const fixture = redact(overview, captured ? Date.now() : FIXTURE_EPOCH_MS);
  fixture._meta.source = source;
  fs.mkdirSync(path.dirname(OUTPUT), {recursive: true});
  fs.writeFileSync(OUTPUT, `${JSON.stringify(fixture, null, 2)}\n`);
  console.log(
    `snapshot-overview-fixture: wrote ${path.relative(process.cwd(), OUTPUT)} ` +
      `(${fixture.devices.length} devices, ${fixture.sessions.length} sessions, ` +
      `${fixture.activity.length} activity rows, source ${source})`
  );
})().catch(error => fail(error.message));
