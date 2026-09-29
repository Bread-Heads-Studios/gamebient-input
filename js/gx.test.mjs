// Unit tests for the pure parts of the web glue. Run: node --test js/gx.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { GX_PAD_BAND, gxComputeFit } from './gx.js';

const PHONE = { vw: 390, vh: 844 };
const near = (a, b) => Math.abs(a - b) < 1e-3;

test('the pad band is 190 CSS px', () => {
  assert.equal(GX_PAD_BAND, 190);
});

test('with nothing reserved the canvas is centred at the largest fit', () => {
  const fit = gxComputeFit({ vw: 1920, vh: 1080, w: 960, h: 720, reserveBottom: 0 });
  assert.equal(fit.align, 'center');
  assert.ok(near(fit.scale, 1.5));
});

test('16:9, 4:3 and 1:1 stay centred on a portrait phone', () => {
  for (const [w, h, scale] of [[1280, 720, 390 / 1280], [960, 720, 390 / 960], [720, 720, 390 / 720]]) {
    const fit = gxComputeFit({ ...PHONE, w, h, reserveBottom: GX_PAD_BAND });
    assert.equal(fit.align, 'center', `${w}x${h}`);
    assert.ok(near(fit.scale, scale), `${w}x${h}`);
  }
});

test('3:4 moves to the top at full width', () => {
  const fit = gxComputeFit({ ...PHONE, w: 720, h: 960, reserveBottom: GX_PAD_BAND });
  assert.equal(fit.align, 'top');
  assert.ok(near(fit.scale, 390 / 720));
  assert.ok(960 * fit.scale <= PHONE.vh - GX_PAD_BAND);
});

test('9:16 moves to the top and shrinks to clear the band', () => {
  const fit = gxComputeFit({ ...PHONE, w: 720, h: 1280, reserveBottom: GX_PAD_BAND });
  assert.equal(fit.align, 'top');
  assert.ok(near(fit.scale, (PHONE.vh - GX_PAD_BAND) / 1280));
});

test('the device pixel ratio does not change the rendered size', () => {
  const at1 = gxComputeFit({ ...PHONE, w: 720, h: 960, reserveBottom: GX_PAD_BAND });
  const at3 = gxComputeFit({ ...PHONE, w: 240, h: 320, reserveBottom: GX_PAD_BAND });
  assert.equal(at1.align, at3.align);
  assert.ok(near(720 * at1.scale, 240 * at3.scale));
});

test('a negative or missing reserve is treated as zero', () => {
  for (const reserveBottom of [undefined, -50]) {
    const fit = gxComputeFit({ ...PHONE, w: 720, h: 960, reserveBottom });
    assert.equal(fit.align, 'center');
  }
});
