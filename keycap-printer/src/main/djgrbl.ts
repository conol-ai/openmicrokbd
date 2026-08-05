import { createCipheriv, createDecipheriv } from "node:crypto";
import type { EngraveJobData, EngravePoint } from "../shared/laser";

const OUTGOING_KEY = Buffer.from("1b6e251826afd8462b17458866c0483e", "hex");
const INCOMING_KEY = Buffer.from("2b7e152628aed2a8abf7158808cf4f3c", "hex");
const AES_IV = Buffer.alloc(16, 1);
const DJGRBL_WORKSPACE_MM = 100;
const DJGRBL_MAX_COORDINATE = 0xffff;
const DJGRBL_UNITS_PER_MM = DJGRBL_MAX_COORDINATE / DJGRBL_WORKSPACE_MM;
const RECORDS_PER_PACKET = 1000;

interface DjGrblRecord {
  type: number;
  x: number;
  y: number;
}

export interface MachinePoint {
  x: number;
  y: number;
}

export interface DjGrblJobPlan {
  setupCommand: Buffer;
  dataCommands: Buffer[];
  endCommand: Buffer;
  startCommand: Buffer;
  progressTotal: number;
  recordCount: number;
}

export function crc16Ccitt(data: Uint8Array): number {
  let crc = 0xffff;
  for (const byte of data) {
    crc ^= byte << 8;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc & 0x8000) !== 0 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
    }
  }
  return crc;
}

export function encodeDjGrblCommand(plaintext: Uint8Array): Buffer {
  const cipher = createCipheriv("aes-128-cbc", OUTGOING_KEY, AES_IV);
  const encrypted = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const frameLength = encrypted.length + 7;
  if (frameLength > 0xffff) throw new Error("DJGRBL command exceeds the protocol frame limit");

  const frameWithoutCrc = Buffer.alloc(frameLength - 2);
  frameWithoutCrc[0] = 0xa1;
  frameWithoutCrc.writeUInt16BE(frameLength, 1);
  encrypted.copy(frameWithoutCrc, 3);
  frameWithoutCrc[frameWithoutCrc.length - 2] = 0x66;
  frameWithoutCrc[frameWithoutCrc.length - 1] = 0x88;

  const frame = Buffer.alloc(frameLength);
  frameWithoutCrc.copy(frame);
  frame.writeUInt16BE(crc16Ccitt(frameWithoutCrc), frameLength - 2);
  return frame;
}

export function decodeDjGrblResponse(frame: Uint8Array): Buffer {
  const data = Buffer.from(frame);
  if (data.length < 21 || data[0] !== 0xb1 || (data.length - 5) % 16 !== 0) {
    throw new Error("Invalid DJGRBL response header");
  }
  if (data.readUInt16BE(1) !== data.length) throw new Error("Invalid DJGRBL response length");
  if (crc16Ccitt(data.subarray(0, -2)) !== data.readUInt16BE(data.length - 2)) {
    throw new Error("Invalid DJGRBL response CRC");
  }

  const decipher = createDecipheriv("aes-128-cbc", INCOMING_KEY, AES_IV);
  decipher.setAutoPadding(false);
  const decrypted = Buffer.concat([decipher.update(data.subarray(3, -2)), decipher.final()]);
  const padding = decrypted.at(-1) ?? 0;
  if (padding > 0 && padding <= 16 && decrypted.subarray(-padding).every((byte) => byte === padding)) {
    return decrypted.subarray(0, -padding);
  }

  // The firmware-identification response is exactly one AES block and is zero padded.
  if (decrypted[0] === 0x11 && decrypted[1] === 0x0b) {
    let end = decrypted.length;
    while (end > 2 && decrypted[end - 1] === 0) end -= 1;
    return decrypted.subarray(0, end);
  }
  return decrypted;
}

export class DjGrblResponseParser {
  private buffer = Buffer.alloc(0);

  push(chunk: Uint8Array): Buffer[] {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    const responses: Buffer[] = [];

    while (this.buffer.length > 0) {
      const start = this.buffer.indexOf(0xb1);
      if (start < 0) {
        this.buffer = Buffer.alloc(0);
        break;
      }
      if (start > 0) this.buffer = this.buffer.subarray(start);
      if (this.buffer.length < 3) break;

      const length = this.buffer.readUInt16BE(1);
      if (length < 5) {
        this.buffer = this.buffer.subarray(1);
        continue;
      }
      if (this.buffer.length < length) break;

      const frame = this.buffer.subarray(0, length);
      this.buffer = this.buffer.subarray(length);
      responses.push(decodeDjGrblResponse(frame));
    }
    return responses;
  }

  reset(): void {
    this.buffer = Buffer.alloc(0);
  }
}

export function buildCenteredDjGrblJob(job: EngraveJobData, center: MachinePoint): DjGrblJobPlan {
  validateJob(job);
  if (!Number.isFinite(center.x) || !Number.isFinite(center.y)) throw new Error("Machine position is unavailable");

  const pathRecords: DjGrblRecord[] = [];
  const coordinates: MachinePoint[] = [];
  let finalPoint: MachinePoint | null = null;

  for (const segment of job.segments) {
    if (segment.length < 2) continue;
    const converted = segment.map((point) => convertPoint(point, center));
    coordinates.push(...converted);
    pathRecords.push(record(0x02, converted[0].x, converted[0].y));
    for (const point of converted.slice(1)) pathRecords.push(record(0x01, point.x, point.y));
    finalPoint = converted.at(-1) ?? finalPoint;
  }
  if (!finalPoint || pathRecords.length < 2) throw new Error("The icon has no engravable vector path");

  // The official path builder emits a final jump record to guarantee laser-off state.
  pathRecords.push(record(0x02, finalPoint.x, finalPoint.y));

  const properties = buildMarkProperties(job);
  const records = [
    ...properties,
    record(0x1d, 0, 0),
    ...pathRecords,
    record(0x1e, 0, 0),
    record(0xff, 0, 0)
  ];

  const dataCommands: Buffer[] = [];
  let sequence = 0;
  for (let offset = 0; offset < records.length; offset += RECORDS_PER_PACKET) {
    const chunk = records.slice(offset, offset + RECORDS_PER_PACKET);
    dataCommands.push(Buffer.concat([Buffer.from([0x10, 0x01, sequence >> 8, sequence & 0xff]), encodeRecords(chunk)]));
    sequence += 1;
  }

  return {
    setupCommand: buildSetupCommand(properties, coordinates),
    dataCommands,
    endCommand: Buffer.from([0x10, 0x01, sequence >> 8, sequence & 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
    startCommand: Buffer.from([0x12, 0x00, 0x00]),
    progressTotal: Math.max(1, records.length - 2),
    recordCount: records.length
  };
}

function buildMarkProperties(job: EngraveJobData): DjGrblRecord[] {
  return [
    valueRecord(0x05, Math.trunc((job.speedPercent / 100) * 0xffff)),
    valueRecord(0x06, 100_000),
    valueRecord(0x08, 0),
    valueRecord(0x09, 200),
    valueRecord(0x07, 0x7fff),
    valueRecord(0x0c, 100),
    valueRecord(0x0e, 1),
    valueRecord(0x03, 20_000),
    valueRecord(0x0a, 2_000),
    valueRecord(0x04, Math.round(job.powerPercent * 10)),
    valueRecord(0x0b, 100),
    valueRecord(0x0f, job.passes)
  ];
}

function buildSetupCommand(properties: DjGrblRecord[], points: MachinePoint[]): Buffer {
  const minX = Math.min(...points.map((point) => point.x));
  const maxX = Math.max(...points.map((point) => point.x));
  const minY = Math.min(...points.map((point) => point.y));
  const maxY = Math.max(...points.map((point) => point.y));
  const previewProperties = [
    valueRecord(0x13, 19_660),
    valueRecord(0x08, 0),
    valueRecord(0x09, 200),
    valueRecord(0x0f, 1),
    valueRecord(0x07, 0x7fff),
    valueRecord(0x0c, 100),
    valueRecord(0x0e, 1),
    valueRecord(0x11, 10_000),
    valueRecord(0x0a, 2_000),
    valueRecord(0x12, 20),
    valueRecord(0x0b, 100)
  ];
  const boundary = [
    record(0x02, minX, minY),
    record(0x01, maxX, minY),
    record(0x01, maxX, maxY),
    record(0x01, minX, maxY),
    record(0x01, minX, minY),
    record(0xff, minX, minY)
  ];
  return Buffer.concat([Buffer.from([0x10, 0x07, 0x00, 0x00]), encodeRecords([...previewProperties, ...properties, ...boundary])]);
}

function convertPoint(point: EngravePoint, center: MachinePoint): MachinePoint {
  const machineX = center.x + point.x;
  const machineY = center.y - point.y;
  const x = Math.trunc(machineX * DJGRBL_UNITS_PER_MM);
  const y = Math.trunc(machineY * DJGRBL_UNITS_PER_MM);
  if (x < 0 || x > DJGRBL_MAX_COORDINATE || y < 0 || y > DJGRBL_MAX_COORDINATE) {
    throw new Error("Icon exceeds the DJGRBL 100 x 100 mm workspace at the current laser position");
  }
  return { x, y };
}

function valueRecord(type: number, value: number): DjGrblRecord {
  const normalized = Math.max(0, Math.min(0xffffffff, Math.trunc(value)));
  return record(type, Math.floor(normalized / 0x10000), normalized & 0xffff);
}

function record(type: number, x: number, y: number): DjGrblRecord {
  return { type: type & 0xff, x: x & 0xffff, y: y & 0xffff };
}

function encodeRecords(records: DjGrblRecord[]): Buffer {
  const output = Buffer.alloc(records.length * 5);
  records.forEach((item, index) => {
    const offset = index * 5;
    output[offset] = item.type;
    output.writeUInt16BE(item.x, offset + 1);
    output.writeUInt16BE(item.y, offset + 3);
  });
  return output;
}

function validateJob(job: EngraveJobData): void {
  if (!Array.isArray(job.segments) || job.segments.length === 0 || job.segments.length > 10_000) {
    throw new Error("Invalid engraving path list");
  }
  let pointCount = 0;
  for (const segment of job.segments) {
    if (!Array.isArray(segment)) throw new Error("Invalid engraving path");
    pointCount += segment.length;
    if (pointCount > 200_000) throw new Error("Engraving job contains too many points");
    for (const point of segment) {
      if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y) || Math.abs(point.x) > 100 || Math.abs(point.y) > 100) {
        throw new Error("Invalid engraving coordinate");
      }
    }
  }
  if (!Number.isFinite(job.powerPercent) || job.powerPercent < 0 || job.powerPercent > 100) throw new Error("Invalid laser power");
  if (!Number.isFinite(job.speedPercent) || job.speedPercent < 1 || job.speedPercent > 100) throw new Error("Invalid engraving speed");
  if (!Number.isInteger(job.passes) || job.passes < 1 || job.passes > 20) throw new Error("Invalid pass count");
}
