import test from "node:test";
import assert from "node:assert/strict";
import { handleEmblemClick, isFullColor, onColorModeChange, spectrumAt, tintPacked, waveDelay } from "../src/graphics-color.ts";

test("the emblem toggles full color on every tenth activation",()=>{
  assert.equal(isFullColor(),false);
  let changes=0;const unsubscribe=onColorModeChange(()=>changes++);
  for(let activation=1;activation<10;activation++)assert.deepEqual(handleEmblemClick(),{fullColor:false,toggled:false});
  assert.deepEqual(handleEmblemClick(),{fullColor:true,toggled:true});
  for(let activation=1;activation<10;activation++)assert.deepEqual(handleEmblemClick(),{fullColor:true,toggled:false});
  assert.deepEqual(handleEmblemClick(),{fullColor:false,toggled:true});
  assert.equal(changes,2);unsubscribe();
});

test("normal mode stays monochrome between waves and tints only their traveling region",()=>{
  assert.equal(spectrumAt(6.5,305.5,0,1000,800,[],false,false).saturation,0);
  const waves=[{x:0,y:0,born:0,strength:1,ellipse:1}];
  assert.equal(spectrumAt(6.5,305.5,0,1000,800,waves,false,false).saturation,.62);
  assert(spectrumAt(6.5,999,0,1000,800,waves,false,false).saturation<1e-10);
  assert.equal(spectrumAt(13,611,0,1000,800,waves,false,false).saturation,0);
});

test("neighboring glyphs and successive frames traverse adjacent hues continuously",()=>{
  const waves=[{x:0,y:0,born:0,strength:1,ellipse:1}];
  const first=spectrumAt(6.5,305.5,0,1000,800,waves,false,false);
  const neighbor=spectrumAt(6.5,313.9,0,1000,800,waves,false,false);
  const next=spectrumAt(6.5+1/30,305.5,0,1000,800,waves,false,false);
  assert(Math.abs(neighbor.hue-first.hue)<.001);
  assert(Math.abs(next.hue-first.hue)<.001);
  assert.equal(tintPacked(0,1),0x0000ff);
  assert.equal(tintPacked(1/3,1),0x00ff00);
  assert.equal(tintPacked(2/3,1),0xff0000);
});

test("reduced motion removes ambient color waves while preserving a chosen static spectrum",()=>{
  const waves=[{x:0,y:0,born:0,strength:1,ellipse:1}];
  assert.equal(spectrumAt(6.5,305.5,0,1000,800,waves,false,true).saturation,0);
  assert.equal(spectrumAt(6.5,305.5,0,1000,800,waves,true,true).saturation,.12);
});

test("ambient waves are separated by twenty-five seconds while the easter egg overlaps successive waves",()=>{
  assert.equal(waveDelay(false,0),25);
  assert.equal(waveDelay(false,1),25);
  assert.equal(waveDelay(true,0),4);
  assert.equal(waveDelay(true,1),6);
  const waves=[{x:0,y:0,born:0,strength:1,ellipse:1}];
  assert(spectrumAt(6.5,305.5,0,1000,800,waves,true,false).saturation>.7);
  assert(spectrumAt(6.5,999,0,1000,800,waves,true,false).saturation<.2);
});