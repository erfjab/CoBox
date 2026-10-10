// Hero scene: the CoBox logo as three glowing slabs, with copied "clips" streaming in
// from the left and landing on the stack, the way CoBox collects everything you copy.
import * as THREE from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";

const canvas = document.getElementById("scene");
const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, powerPreference: "low-power" });
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
renderer.setClearColor(0x16161a);
renderer.outputColorSpace = THREE.SRGBColorSpace;

const scene = new THREE.Scene();
scene.fog = new THREE.Fog(0x16161a, 12, 24);
const camera = new THREE.PerspectiveCamera(35, 1, 0.1, 100);
camera.position.set(0, 0, 11);

scene.add(new THREE.AmbientLight(0xffffff, 0.35));
const key = new THREE.DirectionalLight(0xffffff, 1.6);
key.position.set(-4, 6, 8);
scene.add(key);
const glow = new THREE.PointLight(0x3a86ff, 18, 10, 1.6);
glow.position.set(0, -1, 3);
scene.add(glow);

// ── the logo: same proportions as assets/logo.svg (256-unit grid) ──
const stack = new THREE.Group();
scene.add(stack);
const U = 1 / 40;
const bars = [
  { w: 112, h: 28, y: 70, color: 0x1d293f },
  { w: 132, h: 32, y: 112, color: 0x2a5498 },
  { w: 152, h: 60, y: 170, color: 0x3a86ff },
];
const slabs = bars.map((b, i) => {
  const mat = new THREE.MeshStandardMaterial({
    color: b.color, roughness: 0.35, metalness: 0.1,
    emissive: b.color, emissiveIntensity: i === 2 ? 0.35 : 0.04,
  });
  const m = new THREE.Mesh(new RoundedBoxGeometry(b.w * U, b.h * U, 0.5, 4, Math.min(b.h, 24) * U * 0.42), mat);
  m.position.y = (128 - b.y) * U;
  m.position.z = i * 0.18;
  m.userData.baseY = m.position.y;
  stack.add(m);
  return m;
});
// The two "text lines" on the front slab.
const lineMat = new THREE.MeshStandardMaterial({ color: 0x16161a, roughness: 0.6, transparent: true, opacity: 0.55 });
[[64, 164], [40, 180]].forEach(([w, y], i) => {
  const l = new THREE.Mesh(new RoundedBoxGeometry(w * U, 8 * U, 0.06, 2, 4 * U * 0.9), lineMat.clone());
  l.material.opacity = i ? 0.35 : 0.55;
  l.position.set((72 + w / 2 - 128) * U, (128 - y) * U, slabs[2].position.z + 0.26);
  stack.add(l);
});

// ── clips: small cards drawn like rows of the app ──
const CLIPS = [
  ["url", "https://github.com/erfjab/CoBox"],
  ["code", "fn main() {"],
  ["txt", "meeting moved to thursday"],
  ["img", "image 1920×1080"],
  ["code", "cargo build --profile dist"],
  ["txt", "سلام دنیا"],
  ["pdf", "invoice-2026-10.pdf"],
  ["url", "https://doc.rust-lang.org"],
  ["file", "report.xlsx  notes.md"],
  ["txt", "thanks, that fixed it!"],
  ["vid", "demo-recording.mp4"],
  ["code", "git push origin master"],
  ["aud", "voice-note.m4a"],
  ["txt", "ssh deploy@203.0.113.7"],
];

function cardTexture([tag, text], on) {
  const c = document.createElement("canvas");
  c.width = 640; c.height = 96;
  const g = c.getContext("2d");
  g.fillStyle = on ? "#1d293f" : "#1d1d22";
  g.beginPath(); g.roundRect(2, 2, 636, 92, 14); g.fill();
  g.strokeStyle = on ? "#3a86ff" : "#2c2c34"; g.lineWidth = 3; g.stroke();
  if (on) { g.fillStyle = "#3a86ff"; g.fillRect(2, 18, 6, 60); }
  g.textBaseline = "middle";
  g.font = '500 26px "JetBrains Mono", monospace';
  g.fillStyle = on ? "#3a86ff" : "#9a9aa6";
  g.fillText(tag, 30, 49);
  g.font = '400 30px "JetBrains Mono", monospace';
  g.fillStyle = "#e8e8ec";
  const rtl = /[֐-ࣿ]/.test(text);
  if (rtl) { g.textAlign = "right"; g.fillText(text, 610, 49); } else { g.fillText(text, 118, 49); }
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = 4;
  return t;
}

const cardGeo = new THREE.PlaneGeometry(3.2, 0.48);
let cards = [];
const target = new THREE.Vector3();
const rand = (a, b) => a + Math.random() * (b - a);

function spawn(card, first) {
  const d = card.userData;
  d.t = first ? Math.random() : 0;
  d.speed = rand(0.05, 0.085);
  d.from = new THREE.Vector3(rand(-12, -7), rand(-3.5, 2.5), rand(-6, 0.5));
  d.wobble = rand(0, Math.PI * 2);
  d.spin = rand(-0.5, 0.5);
}

async function makeCards() {
  try { await Promise.race([document.fonts.load('400 30px "JetBrains Mono"'), new Promise((r) => setTimeout(r, 1500))]); } catch {}
  cards = CLIPS.map((clip, i) => {
    const m = new THREE.Mesh(cardGeo, new THREE.MeshBasicMaterial({ map: cardTexture(clip, i % 5 === 0), transparent: true, depthWrite: false }));
    scene.add(m);
    spawn(m, true);
    return m;
  });
}

// ── dust ──
const N = 500;
const pos = new Float32Array(N * 3);
for (let i = 0; i < N; i++) { pos[i * 3] = rand(-14, 14); pos[i * 3 + 1] = rand(-8, 8); pos[i * 3 + 2] = rand(-10, 3); }
const dustGeo = new THREE.BufferGeometry();
dustGeo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
const dust = new THREE.Points(dustGeo, new THREE.PointsMaterial({ color: 0x3a86ff, size: 0.035, transparent: true, opacity: 0.55, depthWrite: false }));
scene.add(dust);

// ── layout & input ──
let wide = true;
function resize() {
  const w = canvas.clientWidth, h = canvas.clientHeight;
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  wide = camera.aspect > 1.05;
  // Desktop: logo on the right of the headline. Phone: above it.
  stack.position.set(wide ? 3.1 : 0, wide ? 0.1 : 3.0, 0);
  stack.scale.setScalar(wide ? 1 : 0.62);
  camera.fov = wide ? 35 : 50;
  camera.updateProjectionMatrix();
}
new ResizeObserver(resize).observe(canvas);
resize();

const pointer = { x: 0, y: 0, sx: 0, sy: 0 };
addEventListener("pointermove", (e) => {
  pointer.x = (e.clientX / innerWidth) * 2 - 1;
  pointer.y = (e.clientY / innerHeight) * 2 - 1;
});

// ── loop ──
let pulse = 0, last = performance.now(), running = true;
function frame(now) {
  const dt = Math.min((now - last) / 1000, 0.05);
  last = now;
  const time = now / 1000;

  pointer.sx += (pointer.x - pointer.sx) * 0.05;
  pointer.sy += (pointer.y - pointer.sy) * 0.05;
  stack.rotation.y = -0.38 + pointer.sx * 0.25 + Math.sin(time * 0.4) * 0.06;
  stack.rotation.x = 0.12 + pointer.sy * 0.12;
  slabs.forEach((s, i) => { s.position.y = s.userData.baseY + Math.sin(time * 1.1 + i * 0.7) * 0.05; });

  pulse = Math.max(0, pulse - dt * 2.2);
  slabs[2].material.emissiveIntensity = 0.35 + pulse * 0.6;
  glow.intensity = 18 + pulse * 30;

  // Clips fly to the top slab, shrink into it, and the stack glows.
  slabs[0].getWorldPosition(target);
  for (const c of cards) {
    const d = c.userData;
    d.t += dt * d.speed;
    if (d.t >= 1) { pulse = 1; spawn(c); }
    const e = 1 - Math.pow(1 - d.t, 2.2);
    c.position.lerpVectors(d.from, target, e);
    c.position.y += Math.sin(d.t * Math.PI) * 0.7 + Math.sin(time + d.wobble) * 0.08;
    const s = d.t > 0.82 ? Math.max(0.01, (1 - d.t) / 0.18) : 1;
    c.scale.setScalar(s * (wide ? 1 : 0.8));
    c.rotation.set(Math.sin(time * 0.7 + d.wobble) * 0.15, 0.35 + d.spin * (1 - e), Math.sin(time * 0.5 + d.wobble) * 0.05);
    c.material.opacity = Math.min(1, d.t * 6) * (d.t > 0.82 ? s : 1);
  }
  dust.rotation.y = time * 0.01;
  dust.position.y = Math.sin(time * 0.2) * 0.2;

  camera.position.x = pointer.sx * 0.6;
  camera.position.y = -pointer.sy * 0.4;
  camera.lookAt(wide ? 0.6 : 0, 0, 0);

  renderer.render(scene, camera);
  if (running && !reduced) requestAnimationFrame(frame);
}

// Only animate while the hero is on screen.
new IntersectionObserver(([e]) => {
  const was = running;
  running = e.isIntersecting && !document.hidden;
  if (running && !was && !reduced) { last = performance.now(); requestAnimationFrame(frame); }
}).observe(canvas);
document.addEventListener("visibilitychange", () => {
  const was = running;
  running = !document.hidden;
  if (running && !was && !reduced) { last = performance.now(); requestAnimationFrame(frame); }
});

makeCards().then(() => requestAnimationFrame(frame));
