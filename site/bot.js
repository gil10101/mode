/*
 * Mode bots: the landing page's animated mascot, drawn on <canvas data-bot="hero|crowd|matrix">.
 *
 * One head = a squircle sampled at 64 angles and closed with Catmull-Rom; two capsule eyes that are
 * holes in it, each drawn in the tangent frame of a unit sphere at its direction (orthographic), so
 * the eye nearer the limb compresses and slides behind it. Hats, glasses and the mustache are placed
 * in the same 3D head frame and projected the same way, so they turn with the head.
 *
 * Eye model, blink schedule and gaze drift after jeremy-prt/bloub (MIT).
 *
 * Every frame is a pure function of time: ModeBot.frame(ctx, scene, t, opts) has no clock and no
 * state; only ModeBot.mount keeps a clock (rAF) and the pointer.
 */
(function () {
  'use strict';

  /* ------------------------------------------------------------------ math (bloub src/bot/math.ts) */

  var TAU = Math.PI * 2;
  var DEG = Math.PI / 180;
  function clamp(v, lo, hi) { lo = lo === undefined ? 0 : lo; hi = hi === undefined ? 1 : hi; return v < lo ? lo : v > hi ? hi : v; }
  function lerp(a, b, t) { return a + (b - a) * t; }
  var easeOutQuint = function (t) { return 1 - Math.pow(1 - t, 5); };
  var easeInOutCubic = function (t) { return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2; };
  var easeInCubic = function (t) { return t * t * t; };
  // ease-out-back whose peak is 1.18 (c^3 / (c+1)^2 * 4/27 = 0.18 gives c = 2.42)
  var BACK_C = 2.42;
  var easeOutBack = function (t) { var y = t - 1; return 1 + (BACK_C + 1) * y * y * y + BACK_C * y * y; };

  /** Periodic 1D noise, seamless over `period` (bloub loopNoise). */
  function loopNoise(t, period, seed) {
    var p = (t / period) * TAU;
    seed = seed || 0;
    return 0.55 * Math.sin(p + seed) + 0.3 * Math.sin(2 * p + seed * 1.7 + 1.1) + 0.15 * Math.sin(3 * p + seed * 2.3 + 2.4);
  }

  /** Deterministic PRNG (mulberry32), bloub createRng. */
  function createRng(seed) {
    var a = seed >>> 0;
    return function () {
      a = (a + 0x6d2b79f5) >>> 0;
      var t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  /* ------------------------------------------------------------------ face life (bloub src/bot/face.ts) */

  var BLINK_RNG = createRng(0x5eed);
  /** Pre-drawn blink schedule: deterministic, stateless. */
  var BLINKS = (function () {
    var out = [];
    var t = 1.4;
    while (t < 900) {
      out.push(t);
      t += 1.9 + BLINK_RNG() * 2.7;
      if (BLINK_RNG() < 0.18) { out.push(t); t += 0.24; }
    }
    return out;
  })();
  var BLINK_DUR = 0.18;

  function blinkLid(t) {
    for (var i = 0; i < BLINKS.length; i++) {
      var start = BLINKS[i];
      if (t < start) break;
      var k = (t - start) / BLINK_DUR;
      if (k >= 0 && k <= 1) return k < 0.45 ? 1 - k / 0.45 : (k - 0.45) / 0.55;
    }
    return 1;
  }
  /** The blink under way at t, or the next one: { at: start, on: t inside it }. */
  function blinkNear(t) {
    for (var i = 0; i < BLINKS.length; i++) {
      if (t <= BLINKS[i] + BLINK_DUR) return { at: BLINKS[i], on: t >= BLINKS[i] };
    }
    return { at: Infinity, on: false };
  }
  /** Blink = vertical squash in screen space around the eye center. */
  function blinkScale(lid) { return 0.06 + 0.94 * clamp(lid); }

  /** Gaze drift from incommensurable periods (deltas in degrees), bloub liveliness. */
  function liveliness(t, wander, out) {
    out.dYaw = (loopNoise(t, 11.3, 0.4) * 5.5 + loopNoise(t, 3.7, 2.1) * 1.6) * wander;
    out.dPitch = (loopNoise(t, 9.1, 1.3) * 4.2 + loopNoise(t, 4.3, 0.7) * 1.3) * wander;
    out.dRoll = loopNoise(t, 13.7, 3.2) * 2.2 * wander;
    out.driftX = loopNoise(t, 7.9, 1.9) * 0.006;
    out.driftY = loopNoise(t, 5.3, 0.3) * 0.007;
    return out;
  }

  /* ------------------------------------------------------------------ our bot, measured */

  /*
   * Measured off assets/logo/final/bot-light-512.png (least-squares fit of a superellipse to the
   * 850 visible outline pixels, top and right edges; the PNG crops the left and the bottom):
   * free fit n = 2.93, sy = 0.985, rms 0.9 px on R = 246 px; the spec's n = 4.2 / sy = 0.88 leaves
   * 7.6 px rms. sy is weakly constrained by the crop (0.94 costs 1.27 px rms), so we keep the
   * head a little wider than tall, as the mark reads, at sy = 0.94.
   */
  var HEAD_N = 2.9;
  var HEAD_SY = 0.94;
  var SAMPLES = 64;
  /*
   * Eyes, same fit (head units, R = half the head's width): capsules 0.157R x 0.376R on screen,
   * centers (-0.38, -0.21) and (+0.52, -0.19). Inverting the sphere model on those four numbers
   * gives yaw 4.7, pitch 12.25, roll 1.0, half-split 25.3 deg (residual < 0.001R); undoing the
   * tangent-frame foreshortening gives the true capsule 0.17 x 0.385.
   */
  var EYE_W = 0.17;
  var EYE_H = 0.385;
  var EYE_SPLIT = 25.3;
  /*
   * The head's rest pitch is 6 (the top hat's brim and band arcs in bot-tophat bulge upward with a
   * sagitta that gives 5-6 deg: we see the hat a little from below). The eyes sit higher than that
   * pitch alone puts them: the other 6.25 deg of the measured 12.25 is the eyes' latitude on the head.
   */
  var EYE_LAT = 6.25;
  var REST = { yaw: 0, pitch: 6, roll: 0 };
  /** The PNG's own pose, for the debug overlay. */
  var DEBUG_GAZE = { yaw: 4.74, pitch: 6, roll: 1 };
  // head center and R in the 512 mark, and in the 200 px variants (the 512 is a 2.56x upscale)
  var PNG_FIT = { 512: { cx: 190.8, cy: 391.6, R: 247.1 }, 200: { cx: 74.5, cy: 153.0, R: 96.5 } };

  var ANG = new Float64Array(SAMPLES), COS = new Float64Array(SAMPLES), SIN = new Float64Array(SAMPLES);
  for (var ai = 0; ai < SAMPLES; ai++) { ANG[ai] = (ai / SAMPLES) * TAU; COS[ai] = Math.cos(ANG[ai]); SIN[ai] = Math.sin(ANG[ai]); }

  /**
   * |x/sx|^n + |y/sy|^n = 1 as r(theta) (bloub superellipseProfile). With sx = 1 the head's widest
   * extent is exactly 1 (2R wide); the radius on the diagonals is larger (1.08), and must stay so.
   */
  function superellipseProfile(n, sx, sy) {
    var out = new Float64Array(SAMPLES);
    for (var i = 0; i < SAMPLES; i++) {
      var c = Math.pow(Math.abs(COS[i] / sx), n);
      var s = Math.pow(Math.abs(SIN[i] / sy), n);
      out[i] = Math.pow(c + s, -1 / n);
    }
    return out;
  }
  var PROFILE = superellipseProfile(HEAD_N, 1, HEAD_SY);

  /** bloub radiusAtAngle: the profile's radius in any direction, interpolated. */
  function radiusAtAngle(angle) {
    var t = ((((angle / TAU) % 1) + 1) % 1) * SAMPLES;
    var i = Math.floor(t);
    return lerp(PROFILE[i % SAMPLES], PROFILE[(i + 1) % SAMPLES], t - i);
  }

  /** The head as a solid of revolution about its up axis: cross-section radius at height h. */
  function headX(h) {
    var a = Math.abs(h / HEAD_SY);
    if (a >= 1) return 0;
    return Math.pow(1 - Math.pow(a, HEAD_N), 1 / HEAD_N);
  }
  /** Height at which the head's cross-section has radius r (where a ring of radius r seats). */
  function seatHeight(r) { return HEAD_SY * Math.pow(1 - Math.pow(r, HEAD_N), 1 / HEAD_N); }

  /* ------------------------------------------------------------------ 3D head frame (bloub eyePoses) */

  function v3(x, y, z) { return [x, y, z]; }
  function dot(a, b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
  function add(a, b, k) { return [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]; }
  function scl(a, k) { return [a[0] * k, a[1] * k, a[2] * k]; }
  function norm(a) { var l = Math.sqrt(dot(a, a)) || 1; return [a[0] / l, a[1] / l, a[2] / l]; }
  /** Turn two orthonormal vectors in their common plane (bloub spin). */
  function spin(u, v, angle) {
    var c = Math.cos(angle), s = Math.sin(angle);
    return [
      [u[0] * c + v[0] * s, u[1] * c + v[1] * s, u[2] * c + v[2] * s],
      [v[0] * c - u[0] * s, v[1] * c - u[1] * s, v[2] * c - u[2] * s]
    ];
  }

  /**
   * Head frame. Screen: x right, y down, z toward the viewer. Positive yaw looks right, positive
   * pitch looks up, positive roll tips the head's right side down.
   */
  function headFrame(g) {
    var f = v3(0, 0, 1), r = v3(1, 0, 0), d = v3(0, 1, 0), p;
    p = spin(f, r, g.yaw * DEG); f = p[0]; r = p[1];
    p = spin(d, f, g.pitch * DEG); d = p[0]; f = p[1];
    p = spin(r, d, g.roll * DEG); r = p[0]; d = p[1];
    // the arrival turn spins the face about the head's own up axis, so the outline does not wobble
    if (g.spin) { p = spin(f, r, -g.spin * DEG); f = p[0]; r = p[1]; }
    var up = scl(d, -1);
    return { f: f, r: r, d: d, up: up, rot: Math.atan2(up[0], -up[1]) };
  }

  /** Where a sphere direction lands on the drawn outline: (v.x, v.y) times the profile's radius there. */
  function onHead(fr, v) {
    var k = radiusAtAngle(Math.atan2(v[1], v[0]) - fr.rot);
    return [v[0] * k, v[1] * k];
  }

  /** The two eyes: sphere direction, tangent frame, depth (bloub eyePoses + a latitude). */
  function eyePoses(fr) {
    var p = spin(fr.d, fr.f, EYE_LAT * DEG);
    var ed = p[0], ef0 = p[1];
    var out = [];
    for (var side = -1; side <= 1; side += 2) {
      var q = spin(ef0, fr.r, EYE_SPLIT * side * DEG);
      var ef = q[0], er = q[1];
      var pos = onHead(fr, ef);
      out.push({ x: pos[0], y: pos[1], a: er[0], b: er[1], c: ed[0], d: ed[1], depth: ef[2], ef: ef, er: er, ed: ed });
    }
    return out;
  }

  /** Point at tangent offset (x, y) from direction m with basis (u, w), wrapped on the sphere (exp map). */
  function wrap(fr, m, u, w, x, y) {
    var s = Math.sqrt(x * x + y * y);
    var dir;
    if (s < 1e-6) dir = m;
    else {
      var k = Math.sin(s) / s;
      dir = [m[0] * Math.cos(s) + (u[0] * x + w[0] * y) * k, m[1] * Math.cos(s) + (u[1] * x + w[1] * y) * k, m[2] * Math.cos(s) + (u[2] * x + w[2] * y) * k];
    }
    var p = onHead(fr, dir);
    return [p[0], p[1], dir[2]];
  }

  /* ------------------------------------------------------------------ color */

  var colourCache = {};
  function parseColour(s) {
    if (colourCache[s]) return colourCache[s];
    var m = /rgba?\(([^)]+)\)/.exec(s), c;
    if (m) { var p = m[1].split(/[\s,\/]+/).filter(Boolean).map(parseFloat); c = [p[0], p[1], p[2]]; }
    else if (s[0] === '#') {
      var h = s.slice(1);
      if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
      c = [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
    } else c = [42, 41, 39];
    colourCache[s] = c;
    return c;
  }
  function mix(a, b, k) {
    var x = parseColour(a), y = parseColour(b);
    return 'rgb(' + Math.round(lerp(x[0], y[0], k)) + ',' + Math.round(lerp(x[1], y[1], k)) + ',' + Math.round(lerp(x[2], y[2], k)) + ')';
  }

  /* ------------------------------------------------------------------ path builders (head units) */

  var PTS = new Float64Array(SAMPLES * 2); // reused body outline points

  /** Closed Catmull-Rom through the 64 profile points (bloub closedPath), rotated by the head's roll. */
  function addBody(ctx, rot) {
    var cr = Math.cos(rot), sr = Math.sin(rot), i;
    for (i = 0; i < SAMPLES; i++) {
      var x = PROFILE[i] * COS[i], y = PROFILE[i] * SIN[i];
      PTS[i * 2] = x * cr - y * sr;
      PTS[i * 2 + 1] = x * sr + y * cr;
    }
    var n = SAMPLES, T = 1 / 6;
    ctx.moveTo(PTS[0], PTS[1]);
    for (i = 0; i < n; i++) {
      var i0 = ((i - 1 + n) % n) * 2, i1 = i * 2, i2 = ((i + 1) % n) * 2, i3 = ((i + 2) % n) * 2;
      ctx.bezierCurveTo(
        PTS[i1] + (PTS[i2] - PTS[i0]) * T, PTS[i1 + 1] + (PTS[i2 + 1] - PTS[i0 + 1]) * T,
        PTS[i2] - (PTS[i3] - PTS[i1]) * T, PTS[i2 + 1] - (PTS[i3 + 1] - PTS[i1 + 1]) * T,
        PTS[i2], PTS[i2 + 1]);
    }
    ctx.closePath();
  }

  /** Capsule in eye-local units mapped through the eye matrix, with the blink squash on screen y. */
  function addEye(ctx, e, k, wScale) {
    var hw = (EYE_W * wScale) / 2, hh = EYE_H / 2, r = Math.min(hw, hh), seg = 12;
    for (var half = 0; half < 2; half++) {
      var cy = half === 0 ? -(hh - r) : hh - r;
      for (var i = 0; i <= seg; i++) {
        var th = Math.PI + (half === 0 ? 0 : Math.PI) + (i / seg) * Math.PI;
        var lx = Math.cos(th) * hw, ly = cy + Math.sin(th) * r;
        var x = e.x + e.a * lx + e.c * ly, y = e.y + (e.b * lx + e.d * ly) * k;
        if (half === 0 && i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
      }
    }
    ctx.closePath();
  }

  var QX = [0, 0, 0, 0], QY = [0, 0, 0, 0];
  var ARC_N = 48;
  /** Projected 3D circle: center c3, in-plane unit axes u3, w3, radius b; consistent winding. */
  function addCircle3(ctx, c3, u3, w3, b) {
    var cross = u3[0] * w3[1] - u3[1] * w3[0];
    var dir = cross >= 0 ? 1 : -1;
    for (var i = 0; i <= ARC_N; i++) {
      var th = (dir * i / ARC_N) * TAU;
      var c = Math.cos(th) * b, s = Math.sin(th) * b;
      var x = c3[0] + u3[0] * c + w3[0] * s, y = c3[1] + u3[1] * c + w3[1] * s;
      if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
    }
    ctx.closePath();
  }
  function addPoly(ctx, xs, ys, n) {
    var area = 0, i;
    for (i = 0; i < n; i++) { var j = (i + 1) % n; area += xs[i] * ys[j] - xs[j] * ys[i]; }
    if (area >= 0) { ctx.moveTo(xs[0], ys[0]); for (i = 1; i < n; i++) ctx.lineTo(xs[i], ys[i]); }
    else { ctx.moveTo(xs[n - 1], ys[n - 1]); for (i = n - 2; i >= 0; i--) ctx.lineTo(xs[i], ys[i]); }
    ctx.closePath();
  }

  /** Axis frame for a solid of revolution: origin o3, axis a3, in-plane e1, e2. */
  function axisInfo(o3, a3, e1, e2) {
    var ax = a3[0], ay = a3[1], l = Math.hypot(ax, ay);
    var a2 = l > 1e-6 ? [ax / l, ay / l] : [0, -1];
    var n2 = [-a2[1], a2[0]];
    // extremes of every slice ellipse along n2 (the screen direction perpendicular to the axis)
    var thStar = Math.atan2(n2[0] * e2[0] + n2[1] * e2[1], n2[0] * e1[0] + n2[1] * e1[1]);
    return { o: o3, a: a3, e1: e1, e2: e2, a2: a2, n2: n2, th: thStar };
  }
  function slicePoint(ax, h, r, th) {
    var c = Math.cos(th) * r, s = Math.sin(th) * r;
    return [ax.o[0] + ax.a[0] * h + ax.e1[0] * c + ax.e2[0] * s, ax.o[1] + ax.a[1] * h + ax.e1[1] * c + ax.e2[1] * s, ax.o[2] + ax.a[2] * h + ax.e1[2] * c + ax.e2[2] * s];
  }
  /**
   * Solid of revolution from slices [[h, r], ...] along the axis. Its outline is the envelope of
   * the slice ellipses: on each slice, the two points where the surface normal
   * (radial - r'(h) * axis) is perpendicular to the view. Where a ring has no such point (seen
   * square on), its extremes across the axis stand in (the spec's cylinder construction). The
   * side polygon through those points, plus every slice ellipse (they all sit inside the
   * envelope), fill the solid; chords between sparse slices would leave scallops instead.
   */
  var SX = [], SY = [];
  function addSolid(ctx, ax, slices) {
    var dense = [], i, k;
    for (i = 0; i < slices.length; i++) {
      if (i === 0) { dense.push(slices[0]); continue; }
      var a = slices[i - 1], b = slices[i], steps = 4;
      for (k = 1; k <= steps; k++) dense.push([lerp(a[0], b[0], k / steps), lerp(a[1], b[1], k / steps)]);
    }
    var n = dense.length;
    var thF = Math.atan2(ax.e2[2], ax.e1[2]), rz = Math.hypot(ax.e1[2], ax.e2[2]);
    SX.length = 0; SY.length = 0;
    var rightX = [], rightY = [], leftX = [], leftY = [];
    for (i = 0; i < n; i++) {
      var h = dense[i][0], r = dense[i][1];
      var i0 = Math.max(0, i - 1), i1 = Math.min(n - 1, i + 1);
      var dh = dense[i1][0] - dense[i0][0];
      var slope = Math.abs(dh) > 1e-9 ? (dense[i1][1] - dense[i0][1]) / dh : 0;
      var c = rz > 1e-6 ? (slope * ax.a[2]) / rz : 2;
      var t1, t2;
      if (Math.abs(c) <= 1) { var ac = Math.acos(c); t1 = thF + ac; t2 = thF - ac; }
      else { t1 = ax.th; t2 = ax.th + Math.PI; }
      var p1 = slicePoint(ax, h, r, t1), p2 = slicePoint(ax, h, r, t2);
      var d1 = p1[0] * ax.n2[0] + p1[1] * ax.n2[1], d2 = p2[0] * ax.n2[0] + p2[1] * ax.n2[1];
      if (d1 < d2) { var tmp = p1; p1 = p2; p2 = tmp; }
      rightX.push(p1[0]); rightY.push(p1[1]); leftX.push(p2[0]); leftY.push(p2[1]);
    }
    for (i = 0; i < n; i++) { SX.push(rightX[i]); SY.push(rightY[i]); }
    for (i = n - 1; i >= 0; i--) { SX.push(leftX[i]); SY.push(leftY[i]); }
    addPoly(ctx, SX, SY, SX.length);
    for (i = 0; i < n; i++) if (dense[i][1] > 1e-4) addCircle3(ctx, add(ax.o, ax.a, dense[i][0]), ax.e1, ax.e2, dense[i][1]);
  }
  /** Dome profile from (h0, r0) up by H: superellipse quarter with exponent m. */
  function dome_(h0, r0, H, m, n) { return dome(h0, r0, H, m, n); }
  function dome(h0, r0, H, m, n) {
    var out = [];
    for (var i = 0; i <= n; i++) {
      var z = Math.sin((i / n) * Math.PI / 2);
      out.push([h0 + H * z, r0 * Math.pow(Math.max(0, 1 - Math.pow(z, m)), 1 / m)]);
    }
    return out;
  }

  /** Front half (facing the viewer) of a slice ellipse, as 3D points. */
  function frontArc(ax, h, r, n) {
    var thF = Math.atan2(ax.e2[2], ax.e1[2]);
    var out = [];
    for (var i = 0; i <= n; i++) out.push(slicePoint(ax, h, r, thF - Math.PI / 2 + (i / n) * Math.PI));
    return out;
  }
  function strokeFrontArc(ctx, ax, h, r, width, color) {
    var pts = frontArc(ax, h, r, 32);
    ctx.beginPath();
    for (var i = 0; i < pts.length; i++) { if (i === 0) ctx.moveTo(pts[i][0], pts[i][1]); else ctx.lineTo(pts[i][0], pts[i][1]); }
    ctx.lineWidth = width; ctx.strokeStyle = color; ctx.lineCap = 'butt'; ctx.stroke();
  }

  /** Head implicit function in the head frame: < 1 inside. */
  function headF(fr, q) {
    var h = dot(q, fr.up);
    var rx = q[0] - fr.up[0] * h, ry = q[1] - fr.up[1] * h, rz = q[2] - fr.up[2] * h;
    var rho = Math.sqrt(rx * rx + ry * ry + rz * rz);
    return Math.pow(rho, HEAD_N) + Math.pow(Math.abs(h / HEAD_SY), HEAD_N);
  }
  function headNormalZ(fr, q) {
    var h = dot(q, fr.up);
    var rx = q[0] - fr.up[0] * h, ry = q[1] - fr.up[1] * h, rz = q[2] - fr.up[2] * h;
    var rho = Math.sqrt(rx * rx + ry * ry + rz * rz) || 1e-9;
    var gr = Math.pow(rho, HEAD_N - 1), gh = (h < 0 ? -1 : 1) * Math.pow(Math.abs(h), HEAD_N - 1) / Math.pow(HEAD_SY, HEAD_N);
    return (rz / rho) * gr + fr.up[2] * gh;
  }
  /**
   * Where a plane (normal n3, offset hc, basis e1, e2) cuts the head surface, visible part only
   * (head normal facing the viewer). Solved by bisection per azimuth.
   */
  function contactCurve(fr, n3, hc, e1, e2) {
    var c0 = scl(n3, hc), N = 64, vis = [], all = [];
    for (var i = 0; i < N; i++) {
      var b = (i / N) * TAU, cb = Math.cos(b), sb = Math.sin(b);
      var dir = [e1[0] * cb + e2[0] * sb, e1[1] * cb + e2[1] * sb, e1[2] * cb + e2[2] * sb];
      var lo = 0, hi = 2;
      for (var it = 0; it < 24; it++) { var mid = (lo + hi) / 2; if (headF(fr, add(c0, dir, mid)) < 1) lo = mid; else hi = mid; }
      var q = add(c0, dir, lo);
      all.push(q); vis.push(headNormalZ(fr, q) > 0);
    }
    // the visible run, in order
    var start = -1;
    for (var j = 0; j < N; j++) if (vis[j] && !vis[(j - 1 + N) % N]) { start = j; break; }
    var out = [];
    if (start < 0) return vis[0] ? all : out;
    for (var k = 0; k < N; k++) { var idx = (start + k) % N; if (!vis[idx]) break; out.push(all[idx]); }
    return out;
  }

  /**
   * The paper hairline where a hat meets the head, at the boundary you actually see: in the hat's
   * axis frame, the lower envelope of the hat's bottom-front edge and the visible part of the
   * head/hat contact curve (seen from below, the head hides the back of the brim, so the contact
   * arc is the boundary; seen from above, the brim's front edge is). Drawn just below that
   * boundary, on the head, clipped to the head's silhouette.
   */
  function hairline(ctx, fr, ax, bottomH, bottomR, plane, width, paper) {
    var A = frontArc(ax, bottomH, bottomR, 40);
    var B = contactCurve(fr, plane.n, plane.hc, plane.e1, plane.e2);
    var la = toLocal(ax, A), lb = toLocal(ax, B);
    if (!la.length) return;
    var x0 = Math.min(la[0][0], lb.length ? lb[0][0] : Infinity), x1 = Math.max(la[la.length - 1][0], lb.length ? lb[lb.length - 1][0] : -Infinity);
    var N = 60, pts = [];
    for (var i = 0; i <= N; i++) {
      var x = lerp(x0, x1, i / N);
      var ya = interp(la, x), yb = interp(lb, x);
      var y = ya === null ? yb : yb === null ? ya : Math.min(ya, yb);
      if (y === null) continue;
      y -= width / 2;
      pts.push([ax.n2[0] * x + ax.a2[0] * y, ax.n2[1] * x + ax.a2[1] * y]);
    }
    ctx.save();
    ctx.beginPath(); addBody(ctx, fr.rot); ctx.clip();
    ctx.beginPath();
    for (var k = 0; k < pts.length; k++) { if (k === 0) ctx.moveTo(pts[k][0], pts[k][1]); else ctx.lineTo(pts[k][0], pts[k][1]); }
    ctx.lineWidth = width; ctx.strokeStyle = paper; ctx.lineJoin = 'round'; ctx.lineCap = 'round'; ctx.stroke();
    ctx.restore();
  }
  function toLocal(ax, pts) {
    var out = [];
    for (var i = 0; i < pts.length; i++) out.push([pts[i][0] * ax.n2[0] + pts[i][1] * ax.n2[1], pts[i][0] * ax.a2[0] + pts[i][1] * ax.a2[1]]);
    out.sort(function (p, q) { return p[0] - q[0]; });
    return out;
  }
  function interp(arr, x) {
    if (!arr.length || x < arr[0][0] || x > arr[arr.length - 1][0]) return null;
    for (var i = 1; i < arr.length; i++) {
      if (x <= arr[i][0]) { var a = arr[i - 1], b = arr[i], d = b[0] - a[0]; return d < 1e-9 ? Math.min(a[1], b[1]) : lerp(a[1], b[1], (x - a[0]) / d); }
    }
    return arr[arr.length - 1][1];
  }

  /* ------------------------------------------------------------------ accessories, measured */

  /*
   * All in head units, measured off the 200 px variants (the head there is the 512 mark scaled by
   * 1/2.56: center (74.5, 153.0), R = 96.5 px), by row/column ink runs (scratchpad tools/acc.py).
   */
  var HAIR = 0.035; // the paper gap between a hat and the head (1.5 to 2 px at R = 96)
  // bot-tophat: crown 1.20R wide (-0.60..0.62) and straight within 0.01 over the visible 0.54R
  // (cropped above); brim reaches 0.88R from the hat axis; brim lower edge at -0.88, upper at -1.04.
  var TOPHAT = { brimR: 0.88, brimT: 0.065, crownR0: 0.60, crownR1: 0.62, crownH: 0.78, band: 0.03 };
  // bot-beanie: fold from screen -0.57 to -0.83 (bulging up: seen from below), fold 0.89R wide at
  // the bottom, 0.80R at the top; dome to -1.21; pompom center -1.31, radius 0.15.
  // The fold hugs the head: its radius is the head's own cross-section plus the knit's thickness,
  // so its ends turn behind the silhouette.
  var BEANIE = { h0: 0.55, h1: 0.81, knit: 0.012, flare: 0.05, domeTop: 1.21, pomH: 1.31, pomR: 0.15 };
  // bot-cap, re-measured (row runs of bot-cap-light.png in head units): the dome's lower edge
  // runs at screen -0.64 (left) to -0.76 (under the bill), seen from below, so its rim sits at
  // h = 0.63 where the head is 0.89R wide; the dome stands well off the head (0.62R half-width at
  // -0.92 where the head is 0.39R) up to -1.14, a 0.48R-high dome (superellipse exponent 1.6
  // fits 0.66 / 0.43 at 0.26 / 0.38 above the rim); button r 0.07 to -1.21. The bill leaves the
  // rim on the right: it shows from x 0.36 (root, under the dome) to a rounded tip at 1.20, i.e.
  // 0.45R past the rim and 0.8R long; it shows 0.27R of its 0.5R width, so its plane is tipped
  // 33 deg toward the viewer about its length; its upper edge is convex. Its centreline climbs
  // from -0.80 at the root to -0.95 at the tip: the bill rises 0.15R, it does not droop, and that
  // rise is what carries its upper edge across the dome (where the PNG draws the paper line).
  var CAP = { h0: 0.63, rimGap: 0.02, domeH: 0.48, domeM: 1.6, button: 0.065, billIn: 0.36, billOut: 0.5, billW: 0.27, billTilt: 33, billRise: 0.15, billSeat: 0.06, billTurn: 25 };
  // billSeat: the root sits 0.06R up the dome (PNG root center -0.80 against the rim's -0.74 there)
  // bot-beret: the contact line runs from (-0.45, -0.94) to (0.90, -0.64): a plane tilted 12.5 deg
  // toward the bot's right at 0.82 from the center; rim from -0.72 to +0.92 along it (radius 0.82,
  // center pushed 0.10 toward +r); top 0.36-0.40 above the plane.
  var BERET = { tilt: 12.5, hc: 0.82, shift: 0.10 };
  // bot-specs (round lenses, T20): ring 0.62R outer diameter centered on each eye, stroke 0.045.
  var SPECS = { r: 0.29, stroke: 0.045, temple: 0.12 };
  // bot-squares: rounded squares 0.58R outer, corner ~0.11R.
  var SQUARES = { side: 0.54, corner: 0.11, stroke: 0.045 };
  // bot-monocle: ring 0.54R outer around the right eye, cord 0.03R hanging from its lower right.
  var MONOCLE = { r: 0.25, stroke: 0.04, cord: 0.025 };
  // bot-mustache: 1.26R wide, 0.30R tall, centered 0.21R below the head center: 19 deg below the
  // head's forward direction. Outline (right half; x from the middle, y down from the head center).
  var MUSTACHE_DROP = 19;
  var MUSTACHE_Y0 = 0.21;
  var MUSTACHE_HALF = [
    [0, 0.122], [0.047, 0.078], [0.126, 0.057], [0.205, 0.073], [0.299, 0.127], [0.393, 0.202],
    [0.472, 0.236], [0.510, 0.236], [0.545, 0.215], [0.566, 0.19], [0.572, 0.162], [0.562, 0.140],
    [0.545, 0.127], [0.522, 0.124], [0.512, 0.112], [0.53, 0.100], [0.556, 0.098], [0.585, 0.105],
    [0.612, 0.13], [0.628, 0.17], [0.627, 0.215], [0.613, 0.255], [0.590, 0.290], [0.551, 0.324],
    [0.504, 0.345], [0.441, 0.358], [0.378, 0.363], [0.299, 0.358], [0.205, 0.342], [0.126, 0.321],
    [0.063, 0.290], [0.031, 0.267], [0, 0.231]
  ];
  var MUSTACHE = (function () {
    var pts = [], i;
    for (i = 0; i < MUSTACHE_HALF.length; i++) pts.push([MUSTACHE_HALF[i][0], MUSTACHE_HALF[i][1] - MUSTACHE_Y0]);
    for (i = MUSTACHE_HALF.length - 2; i > 0; i--) pts.push([-MUSTACHE_HALF[i][0], MUSTACHE_HALF[i][1] - MUSTACHE_Y0]);
    return pts;
  })();

  function hatAxis(fr) { return axisInfo([0, 0, 0], fr.up, fr.r, fr.f); }

  function drawTophat(ctx, fr, ink, paper) {
    var H = TOPHAT, ax = hatAxis(fr);
    var hTop = seatHeight(H.crownR0); // the brim's top sits where the head is as wide as the crown's opening
    var hb = hTop - H.brimT;
    ctx.beginPath();
    addSolid(ctx, ax, [[hb, H.brimR], [hTop, H.brimR]]);
    addSolid(ctx, ax, [[hTop, H.crownR0], [hTop + H.crownH, H.crownR1]]);
    ctx.fillStyle = ink; ctx.fill('nonzero');
    if (fr.up[2] > 0.01) { // the crown's top face, seen from above
      ctx.beginPath(); addCircle3(ctx, add(ax.o, ax.a, hTop + H.crownH), ax.e1, ax.e2, H.crownR1);
      ctx.fillStyle = mix(ink, paper, 0.1); ctx.fill();
    }
    strokeFrontArc(ctx, ax, hTop + H.band * 1.3, lerp(H.crownR0, H.crownR1, 0.05) + 0.004, H.band, paper);
    hairline(ctx, fr, ax, hb, H.brimR, { n: fr.up, hc: hb, e1: fr.r, e2: fr.f }, HAIR, paper);
  }

  function drawBeanie(ctx, fr, ink, paper) {
    var B = BEANIE, ax = hatAxis(fr);
    var r0 = headX(B.h0) + B.knit, r1 = headX(B.h1) + B.flare, rd = r1 - 0.025;
    var domeCol = mix(ink, paper, 0.08);
    var fold = function () { ctx.beginPath(); addSolid(ctx, ax, [[B.h0, r0], [(B.h0 + B.h1) / 2, headX((B.h0 + B.h1) / 2) + (B.knit + B.flare) / 2], [B.h1, r1]]); ctx.fillStyle = ink; ctx.fill(); };
    var cap = function () { ctx.beginPath(); addSolid(ctx, ax, dome(B.h1, rd, B.domeTop - B.h1, 2, 12)); ctx.fillStyle = domeCol; ctx.fill(); };
    // painter's order along the axis: from below, the higher piece is behind
    if (fr.up[2] >= 0) { fold(); cap(); } else { cap(); fold(); }
    strokeFrontArc(ctx, ax, B.h1 - 0.012, r1 - 0.004, 0.03, paper);
    hairline(ctx, fr, ax, B.h0, r0, { n: fr.up, hc: B.h0, e1: fr.r, e2: fr.f }, HAIR, paper);
    // pompom: a sphere, so a disc on screen, with its paper ring
    var c = scl(fr.up, B.pomH);
    ctx.beginPath(); ctx.arc(c[0], c[1], B.pomR, 0, TAU);
    ctx.lineWidth = 0.07; ctx.strokeStyle = paper; ctx.stroke();
    ctx.fillStyle = ink; ctx.fill();
  }

  function drawCap(ctx, fr, ink, paper, side) {
    var C = CAP, ax = hatAxis(fr);
    var r0 = headX(C.h0) + C.rimGap;
    // the bill: a flat tongue leaving the rim at its side point (+r, or -r when mirrored), lying in
    // the plane through the bill's length tipped billTilt toward the viewer's side of the cap
    // the bill points to the side, turned billTurn toward the front (the PNG's upper bill edge
    // crosses the dome, so the bill is not square to the view); across = its width direction
    var bt = C.billTurn * DEG;
    var out = add(scl(fr.r, side * Math.cos(bt)), fr.f, Math.sin(bt));
    var across = add(scl(fr.f, Math.cos(bt)), fr.r, -side * Math.sin(bt)), tl = C.billTilt * DEG;
    var acrossT = add(scl(across, Math.cos(tl)), fr.up, -Math.sin(tl));
    var xs = [], ys = [], n = 36, i;
    var len = C.billIn + C.billOut, flat = 0.55;
    var pt = function (u, v) { // u 0..1 root to tip, v -1..1 across
      var along = r0 - C.billIn + len * u;
      var q = add(add(scl(fr.up, C.h0 + C.billSeat + C.billRise * u * (1.3 - 0.3 * u)), out, along), acrossT, v * C.billW);
      xs.push(q[0]); ys.push(q[1]);
    };
    // outline: back edge root to tip, rounded tip, front edge tip to root
    var halfW = function (u) { return u <= flat ? 1 : Math.sqrt(Math.max(0, 1 - Math.pow((u - flat) / (1 - flat), 2))); };
    // samples crowd toward the rounded tip
    for (i = 0; i <= n; i++) { var u = Math.sin((i / n) * Math.PI / 2); pt(u, -halfW(u)); }
    for (i = n; i >= 0; i--) { var u2 = Math.sin((i / n) * Math.PI / 2); pt(u2, halfW(u2)); }
    var bill = function () {
      // paper halo along the free edges (not the root, which tucks under the dome), then the fill
      ctx.beginPath();
      for (var j = 1; j < xs.length - 1; j++) { if (j === 1) ctx.moveTo(xs[j], ys[j]); else ctx.lineTo(xs[j], ys[j]); }
      ctx.lineWidth = HAIR * 2; ctx.strokeStyle = paper; ctx.lineJoin = 'round'; ctx.lineCap = 'round'; ctx.stroke();
      ctx.beginPath(); addPoly(ctx, xs, ys, xs.length); ctx.fillStyle = ink; ctx.fill();
    };
    var dome = function () {
      ctx.beginPath(); addSolid(ctx, ax, dome_(C.h0, r0, C.domeH, C.domeM, 14)); ctx.fillStyle = ink; ctx.fill();
      hairline(ctx, fr, ax, C.h0, r0, { n: fr.up, hc: C.h0, e1: fr.r, e2: fr.f }, HAIR, paper);
      var bc = scl(fr.up, C.h0 + C.domeH + 0.01);
      ctx.beginPath(); ctx.arc(bc[0], bc[1], C.button, 0, TAU); ctx.fillStyle = ink; ctx.fill();
    };
    // painter's order: a bill turned away from the viewer tucks behind the dome
    if (out[2] < 0) { bill(); dome(); } else { dome(); bill(); }
  }

  function drawBeret(ctx, fr, ink, paper) {
    var B = BERET, t = B.tilt * DEG;
    var nb = add(scl(fr.up, Math.cos(t)), fr.r, Math.sin(t));
    var e1 = add(scl(fr.r, Math.cos(t)), fr.up, -Math.sin(t));
    var o = add(scl(nb, B.hc), e1, B.shift);
    var ax = axisInfo(o, nb, e1, fr.f);
    // a puffed rim (max 0.855R just above the band) rolling into a low crown
    var prof = [[0, 0.77], [0.02, 0.81], [0.05, 0.84], [0.085, 0.855], [0.12, 0.85], [0.16, 0.82], [0.21, 0.75], [0.26, 0.63], [0.30, 0.48], [0.33, 0.32], [0.35, 0.15], [0.356, 0]];
    ctx.beginPath(); addSolid(ctx, ax, prof); ctx.fillStyle = ink; ctx.fill();
    hairline(ctx, fr, ax, 0, 0.78, { n: nb, hc: B.hc, e1: e1, e2: fr.f }, HAIR, paper);
    // stem: a short cylinder on the crown, along the beret's own normal
    var sa = axisInfo(o, nb, e1, fr.f);
    ctx.beginPath(); addSolid(ctx, sa, [[0.33, 0.045], [0.42, 0.04]]); ctx.fillStyle = ink; ctx.fill();
  }

  function eyePoint(e, lx, ly) { return [e.x + e.a * lx + e.c * ly, e.y + e.b * lx + e.d * ly]; }

  function drawSpecs(ctx, fr, eyes, paper, square) {
    var S = square ? SQUARES : SPECS, i;
    ctx.strokeStyle = paper; ctx.lineWidth = S.stroke; ctx.lineJoin = 'round'; ctx.lineCap = 'round';
    var half = square ? SQUARES.side / 2 : SPECS.r;
    for (i = 0; i < 2; i++) {
      var e = eyes[i];
      if (e.depth <= 0.02) continue;
      ctx.beginPath();
      if (square) {
        var hs = SQUARES.side / 2, cr = SQUARES.corner, seg = 6, c, k;
        var corners = [[hs - cr, -hs + cr, -Math.PI / 2], [hs - cr, hs - cr, 0], [-hs + cr, hs - cr, Math.PI / 2], [-hs + cr, -hs + cr, Math.PI]];
        for (c = 0; c < 4; c++) for (k = 0; k <= seg; k++) {
          var th = corners[c][2] + (k / seg) * Math.PI / 2;
          var p = eyePoint(e, corners[c][0] + Math.cos(th) * cr, corners[c][1] + Math.sin(th) * cr);
          if (c === 0 && k === 0) ctx.moveTo(p[0], p[1]); else ctx.lineTo(p[0], p[1]);
        }
      } else {
        for (var j = 0; j <= 48; j++) {
          var a = (j / 48) * TAU, q = eyePoint(e, Math.cos(a) * SPECS.r, Math.sin(a) * SPECS.r);
          if (j === 0) ctx.moveTo(q[0], q[1]); else ctx.lineTo(q[0], q[1]);
        }
      }
      ctx.closePath(); ctx.stroke();
      // temple stub, outward along the head's right
      var side = i === 0 ? -1 : 1;
      var endDepth = e.depth + e.er[2] * side * (half + SPECS.temple);
      if (endDepth > 0.05) {
        var p0 = eyePoint(e, side * half, 0), p1 = eyePoint(e, side * (half + SPECS.temple), 0);
        ctx.beginPath(); ctx.moveTo(p0[0], p0[1]); ctx.lineTo(p1[0], p1[1]); ctx.stroke();
      }
    }
    if (eyes[0].depth > 0.02 && eyes[1].depth > 0.02) {
      var b0 = eyePoint(eyes[0], half, 0), b1 = eyePoint(eyes[1], -half, 0);
      ctx.beginPath(); ctx.moveTo(b0[0], b0[1]); ctx.lineTo(b1[0], b1[1]); ctx.stroke();
    }
  }

  var CORD = [[0.16, 0.19], [0.235, 0.235], [0.255, 0.3], [0.258, 0.38], [0.258, 0.46], [0.258, 0.53]];
  function drawMonocle(ctx, fr, eyes, paper) {
    var e = eyes[1], M = MONOCLE;
    if (e.depth <= 0.02) return;
    ctx.strokeStyle = paper; ctx.lineWidth = M.stroke; ctx.lineJoin = 'round'; ctx.lineCap = 'round';
    ctx.beginPath();
    for (var j = 0; j <= 48; j++) { var a = (j / 48) * TAU, q = eyePoint(e, Math.cos(a) * M.r, Math.sin(a) * M.r); if (j === 0) ctx.moveTo(q[0], q[1]); else ctx.lineTo(q[0], q[1]); }
    ctx.closePath(); ctx.stroke();
    // cord: from the ring's lower right, draped along the sphere surface, as quadratic segments
    var pts = [];
    for (var i = 0; i < CORD.length; i++) {
      var p = wrap(fr, e.ef, e.er, e.ed, CORD[i][0], CORD[i][1]);
      if (p[2] <= 0.05) break;
      pts.push(p);
    }
    if (pts.length < 2) return;
    ctx.lineWidth = M.cord;
    ctx.beginPath(); ctx.moveTo(pts[0][0], pts[0][1]);
    for (var k = 1; k < pts.length - 1; k++) ctx.quadraticCurveTo(pts[k][0], pts[k][1], (pts[k][0] + pts[k + 1][0]) / 2, (pts[k][1] + pts[k + 1][1]) / 2);
    ctx.lineTo(pts[pts.length - 1][0], pts[pts.length - 1][1]);
    ctx.stroke();
  }

  function mustacheFrame(fr) {
    var p = spin(fr.d, fr.f, -MUSTACHE_DROP * DEG); // forward turned toward down
    return { m: p[1], dm: p[0] };
  }
  function mustachePoints(fr) {
    var mf = mustacheFrame(fr), out = [];
    if (mf.m[2] < 0.25) return out;
    for (var i = 0; i < MUSTACHE.length; i++) out.push(wrap(fr, mf.m, fr.r, mf.dm, MUSTACHE[i][0], MUSTACHE[i][1]));
    return out;
  }
  function drawMustache(ctx, fr, paper) {
    var pts = mustachePoints(fr);
    if (!pts.length) return;
    ctx.beginPath();
    // smooth through the measured points (midpoint quadratics)
    var n = pts.length;
    ctx.moveTo((pts[n - 1][0] + pts[0][0]) / 2, (pts[n - 1][1] + pts[0][1]) / 2);
    for (var i = 0; i < n; i++) { var a = pts[i], b = pts[(i + 1) % n]; ctx.quadraticCurveTo(a[0], a[1], (a[0] + b[0]) / 2, (a[1] + b[1]) / 2); }
    ctx.closePath(); ctx.fillStyle = paper; ctx.fill();
  }

  /* ------------------------------------------------------------------ one bot */

  /**
   * bot: { x, y: head center (px); R; gaze; lid; acc: [names]; sx, sy, pivotY: squash about the
   * floor contact; twist: cap visor side }.
   */
  function drawBot(ctx, bot, ink, paper, debugAxes) {
    var fr = headFrame(bot.gaze);
    var eyes = eyePoses(fr);
    ctx.save();
    if (bot.sx !== 1 || bot.sy !== 1) {
      ctx.translate(bot.x, bot.pivotY); ctx.scale(bot.sx, bot.sy); ctx.translate(-bot.x, -bot.pivotY);
    }
    ctx.translate(bot.x, bot.y); ctx.scale(bot.R, bot.R);

    // body with the eyes as holes: clip to body XOR eyes, fill the body (body minus eyes)
    var k = blinkScale(bot.lid);
    ctx.beginPath(); addBody(ctx, fr.rot);
    for (var i = 0; i < 2; i++) if (eyes[i].depth > 0.02) addEye(ctx, eyes[i], k, clamp(eyes[i].depth / 0.12, 0.3, 1));
    ctx.save(); ctx.clip('evenodd');
    ctx.beginPath(); addBody(ctx, fr.rot); ctx.fillStyle = ink; ctx.fill();
    ctx.restore();

    var acc = bot.acc || [];
    for (var j = 0; j < acc.length; j++) {
      switch (acc[j]) {
        case 'tophat': drawTophat(ctx, fr, ink, paper); break;
        case 'beanie': drawBeanie(ctx, fr, ink, paper); break;
        case 'beret': drawBeret(ctx, fr, ink, paper); break;
        case 'cap': drawCap(ctx, fr, ink, paper, bot.twist || 1); break;
        case 'specs': drawSpecs(ctx, fr, eyes, paper, false); break;
        case 'squares': drawSpecs(ctx, fr, eyes, paper, true); break;
        case 'monocle': drawMonocle(ctx, fr, eyes, paper); break;
        case 'mustache': drawMustache(ctx, fr, paper); break;
      }
    }
    if (debugAxes) {
      var axes = [[fr.f, '#e5484d'], [fr.r, '#30a46c'], [fr.up, '#0090ff']];
      ctx.lineWidth = 0.012;
      for (var a = 0; a < 3; a++) { ctx.beginPath(); ctx.moveTo(0, 0); ctx.lineTo(axes[a][0][0] * 1.3, axes[a][0][1] * 1.3); ctx.strokeStyle = axes[a][1]; ctx.stroke(); }
    }
    ctx.restore();
    return { fr: fr, eyes: eyes };
  }

  /* ------------------------------------------------------------------ pointer (bloub setLook semantics) */

  var LOOK_MORPH = 0.24;
  function createPointer() { return { px: 0, py: 0, qx: 0, qy: 0, at: -10, m0: 0, m1: 0, mAt: -10, mDur: 0.24, sx: 1, sy: 1 }; }
  function pointerAt(P, t) {
    if (!P) return { x: 0, y: 0, m: 0, sx: 1, sy: 1 };
    var e = easeOutQuint(clamp((t - P.at) / LOOK_MORPH));
    var em = easeOutQuint(clamp((t - P.mAt) / P.mDur));
    return { x: lerp(P.qx, P.px, e), y: lerp(P.qy, P.py, e), m: lerp(P.m0, P.m1, em), sx: P.sx, sy: P.sy };
  }
  /** Retarget from the value on screen now, never from the previous target. */
  function setPointer(P, x, y, t) {
    var cur = pointerAt(P, t);
    if (cur.m < 0.01) { P.qx = x; P.qy = y; } else { P.qx = cur.x; P.qy = cur.y; }
    P.px = x; P.py = y; P.at = t;
    if (P.m1 !== 1) { P.m0 = cur.m; P.m1 = 1; P.mAt = t; P.mDur = 0.24; }
  }
  function releasePointer(P, t) {
    var cur = pointerAt(P, t);
    P.m0 = cur.m; P.m1 = 0; P.mAt = t; P.mDur = 0.6;
  }

  var YAW_MAX = 18, PITCH_MAX = 14, PITCH = 6;
  /** bloub lookTarget, without the settings turn: nx, ny in -1..1 (right, down positive). */
  function lookTarget(nx, ny) { return { yaw: nx * YAW_MAX, pitch: PITCH - ny * PITCH_MAX }; }

  /* ------------------------------------------------------------------ scenes */

  var LIFE = { dYaw: 0, dPitch: 0, dRoll: 0, driftX: 0, driftY: 0 };
  var STILL = { dYaw: 0, dPitch: 0, dRoll: 0, driftX: 0, driftY: 0 }; // no drift: the event-driven gaze bot holds still between events
  var TOUR_TIME = 1.5;

  function heroBots(t, env) {
    var w = env.w, h = env.h, R = 0.36 * Math.min(w, h);
    var bot = { x: w / 2, y: h * (env.heroY || 0.5), R: R, sx: 1, sy: 1, pivotY: h / 2, lid: 1, acc: env.acc || [] };
    if (env.debug) { bot.gaze = DEBUG_GAZE; return [bot]; }
    if (env.reduced) { bot.gaze = REST; return [bot]; }
    var p = pointerAt(env.pointer, t);
    var m = p.m;
    var nx = clamp((p.x - w / 2) / p.sx, -1, 1), ny = clamp((p.y - h / 2) / p.sy, -1, 1);
    var look = lookTarget(nx, ny);
    var wander = lerp(1, 0.35, m);
    var life = env.noLife ? STILL : liveliness(t, wander, LIFE);
    // the arrival tour: a full turn taken on the way, ease-in-out (bloub tourLook)
    var spinDeg = env.noTour ? 0 : 360 * (1 - easeInOutCubic(clamp(t / TOUR_TIME)));
    bot.gaze = {
      yaw: lerp(REST.yaw, look.yaw, m) + life.dYaw,
      pitch: lerp(REST.pitch, look.pitch, m) + life.dPitch,
      roll: REST.roll + life.dRoll,
      spin: spinDeg
    };
    bot.lid = blinkLid(t);
    bot.x += life.driftX * R; bot.y += life.driftY * R;
    return [bot];
  }

  var FLOOR = 0.78;
  var CROWD_R = 0.15;
  var SIZE = [1.04, 0.95, 1.07, 0.96, 1.08, 0.93]; // +-8 % per bot
  // Targets moved out from 0.11/0.23/0.35 so the heads at R = 0.15h (0.05w on a 1550/520 stage)
  // still leave the middle 24 % of the width empty and do not touch each other.
  var CROWD = [
    // side, target (fraction of w), delay, accessories
    { side: -1, x: 0.06, delay: 0, acc: ['tophat'] },
    { side: -1, x: 0.18, delay: 0.45, acc: ['beret', 'mustache'] },
    { side: -1, x: 0.30, delay: 0.9, acc: ['specs'] },
    { side: 1, x: 0.70, delay: 0.2, acc: ['beanie'] },
    { side: 1, x: 0.82, delay: 0.65, acc: ['monocle'] },
    { side: 1, x: 0.95, delay: 1.1, acc: ['cap'] }
  ];
  // Two ballistic hops under one gravity: apex 0.32h then 0.14h. g = 8 * apex / T^2 with T1 = 0.9 s.
  var HOP1_APEX = 0.32, HOP2_APEX = 0.14, HOP1_T = 0.9, GRAV = (8 * HOP1_APEX) / (HOP1_T * HOP1_T);
  var HOP2_T = 2 * Math.sqrt((2 * HOP2_APEX) / GRAV);
  var SQUASH_HOLD = 0.12, SQUASH_REC = 0.3;
  var HOP_LAND = HOP1_T + SQUASH_HOLD + HOP2_T; // time of the final landing after a bot's delay

  function crowdBots(t, env) {
    var w = env.w, h = env.h, floor = FLOOR * h, out = [];
    var P = pointerAt(env.pointer, t);
    for (var i = 0; i < CROWD.length; i++) {
      var c = CROWD[i], R = CROWD_R * h * SIZE[i];
      var x0 = c.side < 0 ? -2 * R : w + 2 * R, xt = c.x * w, x1 = x0 + (xt - x0) * 0.62;
      var local = env.reduced ? 10 : t - c.delay;
      var x = x0, lift = 0, sx = 1, sy = 1;
      if (local < 0) x = x0;
      else if (local < HOP1_T) {
        var u = local / HOP1_T;
        x = lerp(x0, x1, u); lift = 4 * HOP1_APEX * h * u * (1 - u);
        sy = 1 + 0.05 * Math.sin(Math.PI * u); sx = 1 / Math.sqrt(sy);
      } else if (local < HOP1_T + SQUASH_HOLD) {
        x = x1; sx = 1.12; sy = 0.88;
      } else if (local < HOP_LAND) {
        var l2 = local - HOP1_T - SQUASH_HOLD, u2 = l2 / HOP2_T;
        var q = easeOutQuint(clamp(l2 / SQUASH_REC));
        x = lerp(x1, xt, u2); lift = 4 * HOP2_APEX * h * u2 * (1 - u2);
        var st = 1 + 0.05 * Math.sin(Math.PI * u2);
        sy = lerp(0.88, 1, q) * st; sx = lerp(1.12, 1, q) / Math.sqrt(st);
      } else {
        x = xt;
        var l3 = local - HOP_LAND;
        if (l3 < SQUASH_HOLD) { sx = 1.12; sy = 0.88; }
        else { var q3 = easeOutQuint(clamp((l3 - SQUASH_HOLD) / SQUASH_REC)); sx = lerp(1.12, 1, q3); sy = lerp(0.88, 1, q3); }
      }
      var landed = local - HOP_LAND;
      var face = easeOutQuint(clamp(landed / 0.5));
      var yaw = -c.side * lerp(24, 12, face); // positive yaw looks right: left bots look toward the center
      var wander = 0.8 * face;
      var life = liveliness(t + i * 0.7, wander, LIFE);
      var cy = floor - lift - HEAD_SY * R;
      var nx = clamp((P.x - x) / (w / 2), -1, 1), ny = clamp((P.y - cy) / (h / 2), -1, 1);
      var lk = lookTarget(nx, ny), pm = 0.35 * P.m;
      out.push({
        x: x, y: cy, R: R, sx: sx, sy: sy, pivotY: floor - lift, acc: c.acc, twist: -c.side, // visor turned toward the center
        lid: landed > 0 ? blinkLid(t + i * 0.7) : 1,
        gaze: {
          yaw: lerp(yaw, lk.yaw, pm) + life.dYaw,
          pitch: lerp(4, lk.pitch, pm) + life.dPitch,
          roll: -c.side * 5 + life.dRoll
        }
      });
    }
    return out;
  }

  /* matrix: the owner's second version, after the root multiply_animation-1.mp4 */
  var MATRIX_ACC = [[], ['specs'], ['mustache'], ['monocle'], ['specs'], []];
  var MATRIX_R = 0.17, MATRIX_CELLS = 8, MATRIX_SQUARE = 0.76; // R = 0.17h, pitch R/8 (16 cells across), squares 0.76 pitch
  var MATRIX_START = [0, 0.35, 0.7, 0.18, 0.53, 0.88];
  var MATRIX_PERIOD = 7.5, MATRIX_COLLAPSE = 5.3, MATRIX_COLLAPSE_DUR = 0.8, POP = 0.22;
  var SACCADES = MATRIX_START.map(function (_, i) {
    var rng = createRng(0xa11 + i * 7919), out = [], t = 0.6 + rng() * 0.8;
    while (t < 900) { out.push([t, -20 + 40 * rng(), -8 + 22 * rng()]); t += 1.6 + rng() * 1.0; }
    return out;
  });
  function saccadeAt(i, t) {
    var s = SACCADES[i], k = -1;
    for (var j = 0; j < s.length; j++) { if (s[j][0] > t) break; k = j; }
    if (k < 0) return { yaw: REST.yaw, pitch: REST.pitch };
    var prev = k > 0 ? s[k - 1] : [0, REST.yaw, REST.pitch];
    var e = easeOutQuint(clamp((t - s[k][0]) / 0.28));
    return { yaw: lerp(prev[1], s[k][1], e), pitch: lerp(prev[2], s[k][2], e) };
  }

  function capsuleSdf(lx, ly) {
    var hw = EYE_W / 2, hh = EYE_H / 2 - EYE_W / 2;
    var dy = Math.max(Math.abs(ly) - hh, 0);
    return Math.sqrt(lx * lx + dy * dy) - hw;
  }
  function segDist(px, py, ax, ay, bx, by) {
    var dx = bx - ax, dy = by - ay, l = dx * dx + dy * dy || 1e-9;
    var u = clamp(((px - ax) * dx + (py - ay) * dy) / l);
    return Math.hypot(px - ax - dx * u, py - ay - dy * u);
  }
  function polyDistOpen(pts, x, y) {
    var d = Infinity;
    for (var i = 1; i < pts.length; i++) d = Math.min(d, segDist(x, y, pts[i - 1][0], pts[i - 1][1], pts[i][0], pts[i][1]));
    return d;
  }
  function polyDist(pts, x, y) {
    var d = Infinity;
    for (var i = 0, j = pts.length - 1; i < pts.length; j = i++) d = Math.min(d, segDist(x, y, pts[j][0], pts[j][1], pts[i][0], pts[i][1]));
    return d;
  }
  function inPoly(pts, x, y) {
    var inside = false;
    for (var i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      var xi = pts[i][0], yi = pts[i][1], xj = pts[j][0], yj = pts[j][1];
      if ((yi > y) !== (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
    }
    return inside;
  }

  /**
   * env.layout (from the page) may override the preview's stage: targets (six fractions of w),
   * floor (fraction of h), r (fraction of h), rMax (px) and rW (fraction of w, so narrow strips
   * do not make neighbors touch).
   */
  function matrixBots(t, env) {
    var L = env.layout || {}, w = env.w, h = env.h, floor = (L.floor || FLOOR) * h, out = [];
    var Rm = Math.min((L.r || MATRIX_R) * h, L.rMax || Infinity, (L.rW || Infinity) * w);
    for (var i = 0; i < 6; i++) {
      var c = CROWD[i], R = Rm; // one size: at 0.17h the +-8 % spread would make neighbors touch
      var sac = env.reduced ? REST : saccadeAt(i, t);
      var life = liveliness(t + i * 0.9, env.reduced ? 0 : 1.6, LIFE);
      out.push({
        x: (L.targets ? L.targets[i] : c.x) * w, y: floor - HEAD_SY * R, R: R, acc: MATRIX_ACC[i], start: MATRIX_START[i],
        lid: env.reduced ? 1 : blinkLid(t + i * 0.7),
        gaze: { yaw: sac.yaw + life.dYaw, pitch: sac.pitch + life.dPitch, roll: life.dRoll }
      });
    }
    return out;
  }

  /**
   * The hero as the square matrix: one plain head, pitch R/8, multiplying in from the center over
   * about 1 s (the whole head, 1.25R, at 1.6R/s plus the 0.22 s pop), no collapse; the gaze is the
   * hero's own (pointer, drift, blinks), without the arrival turn.
   */
  var HERO_MATRIX_V = 1.6;
  // cursor play on the hero: squares shy away from the pointer within REPEL (head radii), and a click
  // sends a springy ripple out from where it landed (BOOP s long) while the eyes squint
  var REPEL = 0.62, BOOP = 1.4, BOOP_SQUINT = 0.9;
  function heroMatrix(t, env) {
    env.noTour = true;
    var b = heroBots(t, env)[0];
    b.acc = []; b.start = 0;
    // a boop squints the eyes shut for a moment, like a pleased smile
    var P = env.pointer;
    if (P && P.boops && !env.reduced) for (var i = 0; i < P.boops.length; i++) {
      var a = t - P.boops[i].t;
      if (a >= 0 && a < BOOP_SQUINT) b.lid = Math.min(b.lid, a < 0.08 ? 1 - a / 0.08 * 0.85 : 0.15 + 0.85 * easeInOutCubic(clamp((a - 0.08 - 0.35) / (BOOP_SQUINT - 0.43))));
    }
    return { bots: [b], tl: env.reduced ? 10 : t, v: HERO_MATRIX_V, collapse: false, interact: !env.reduced && env.play };
  }
  function drawMatrix(ctx, t, env, ink, paper, given) {
    var bots = given ? given.bots : matrixBots(t, env);
    var tl = given ? given.tl : env.reduced ? 3 : ((t % MATRIX_PERIOD) + MATRIX_PERIOD) % MATRIX_PERIOD;
    var speed = given ? given.v : 2.4, collapse = given ? given.collapse : true;
    // hold mode (the page): multiply in at hold.inAt, stay alive, collapse (0.8 s, center outward,
    // no stagger) only from hold.outAt; nothing before the first arrival
    var hold = !given && env.hold ? env.hold : null;
    if (hold) { if (env.reduced) hold = null; else if (!(hold.inAt <= t)) return bots; else tl = t - hold.inAt; }
    var inkPath = [], paperPath = [];
    var play = !!(given && given.interact && env.pointer), pp = play ? pointerAt(env.pointer, t) : null, boops = play ? (env.pointer.boops || []) : [];
    for (var b = 0; b < bots.length; b++) {
      var bot = bots[b], R = bot.R, p = R / MATRIX_CELLS, side = MATRIX_SQUARE * p;
      var fr = headFrame(bot.gaze), eyes = eyePoses(fr), k = blinkScale(bot.lid);
      var acc = bot.acc, hasSpecs = acc.indexOf('specs') >= 0, hasMono = acc.indexOf('monocle') >= 0, hasMus = acc.indexOf('mustache') >= 0;
      var mus = hasMus ? mustachePoints(fr) : null;
      var cord = null;
      if (hasMono && eyes[1].depth > 0.02) {
        cord = [];
        for (var ci = 0; ci < CORD.length; ci++) { var cp = wrap(fr, eyes[1].ef, eyes[1].er, eyes[1].ed, CORD[ci][0], CORD[ci][1]); if (cp[2] <= 0.05) break; cord.push(cp); }
        if (cord.length < 2) cord = null;
      }
      // inverse eye matrices (blink included)
      var inv = eyes.map(function (e) {
        var a = e.a, bb = e.b * k, c = e.c, d = e.d * k, det = a * d - bb * c || 1e-9;
        return { ia: d / det, ib: -bb / det, ic: -c / det, id: a / det, s: Math.sqrt(Math.abs(det)), x: e.x, y: e.y, vis: e.depth > 0.02 };
      });
      var invPlain = eyes.map(function (e) {
        var det = e.a * e.d - e.b * e.c || 1e-9;
        return { ia: e.d / det, ib: -e.b / det, ic: -e.c / det, id: e.a / det, s: Math.sqrt(Math.abs(det)), x: e.x, y: e.y, vis: e.depth > 0.02 };
      });
      var bridge = hasSpecs && eyes[0].depth > 0.02 && eyes[1].depth > 0.02 ? [eyePoint(eyes[0], SPECS.r, 0), eyePoint(eyes[1], -SPECS.r, 0)] : null;
      var maxD = 1.25 * R;
      var lt = tl - bot.start;
      var span = Math.ceil((1.08 * R) / p);
      for (var gy = -span; gy <= span; gy++) for (var gx = -span; gx <= span; gx++) {
        var hx = (gx * p) / R, hy = (gy * p) / R; // head units
        var dist = Math.hypot(hx, hy);
        // arrival (center outward at v = 2.4R/s, ease-out-back pop) and collapse in the same order
        var dpx = dist * R;
        var kin = (lt - dpx / (speed * R)) / POP;
        if (kin <= 0) continue;
        var kout = hold ? (hold.outAt !== null && hold.outAt !== undefined ? (t - hold.outAt - (dpx / maxD) * (MATRIX_COLLAPSE_DUR - POP)) / POP : 0)
          : collapse ? (lt - MATRIX_COLLAPSE - (dpx / maxD) * (MATRIX_COLLAPSE_DUR - POP)) / POP : 0;
        if (kout >= 1) continue;
        var arrive = (kin >= 1 ? 1 : easeOutBack(kin)) * (kout > 0 ? 1 - easeInCubic(kout) : 1);
        // coverage of the head: signed distance to the outline along the radius
        var rho = radiusAtAngle(Math.atan2(hy, hx) - fr.rot);
        var cover = clamp(0.5 + ((rho - dist) * R) / p);
        // eyes are holes with a half-cell soft edge; zero just outside the capsule, so an eye that
        // falls between two columns still empties both
        var eyeF = 1;
        for (var e = 0; e < 2; e++) {
          var iv = inv[e];
          if (!iv.vis) continue;
          var rx = hx - iv.x, ry = hy - iv.y;
          var lx = iv.ia * rx + iv.ic * ry, ly = iv.ib * rx + iv.id * ry;
          eyeF *= clamp((capsuleSdf(lx, ly) * iv.s * R - 0.05 * p) / (0.5 * p));
        }
        // feature cells: a feature's outline within half a cell of the center. Drawn full scale in a
        // mid tone whatever the head coverage; an eye hole stays a hole.
        var isPaper = false;
        if (eyeF > 0.5 && cover > 0.05) {
          if (hasSpecs || hasMono) {
            for (var q = hasSpecs ? 0 : 1; q < 2 && !isPaper; q++) {
              var ip = invPlain[q];
              if (!ip.vis) continue;
              var ux = hx - ip.x, uy = hy - ip.y;
              var mx = ip.ia * ux + ip.ic * uy, my = ip.ib * ux + ip.id * uy;
              var ringR = hasSpecs ? SPECS.r : MONOCLE.r;
              if (Math.abs(Math.hypot(mx, my) - ringR) * ip.s * R < 0.5 * p) isPaper = true;
            }
            if (!isPaper && bridge && segDist(hx, hy, bridge[0][0], bridge[0][1], bridge[1][0], bridge[1][1]) * R < 0.5 * p) isPaper = true;
          }
          if (!isPaper && cord && polyDistOpen(cord, hx, hy) * R < 0.5 * p) isPaper = true;
          if (!isPaper && mus && mus.length && (inPoly(mus, hx, hy) || polyDist(mus, hx, hy) * R < 0.5 * p)) isPaper = true;
        }
        var sc = isPaper ? arrive : arrive * cover * eyeF;
        if (sc <= 0.02) continue;
        var cx0 = bot.x + gx * p, cy0 = bot.y + gy * p, ox = 0, oy = 0, grow = 1;
        if (play) {
          if (pp.m > 0.01) {
            var vx = cx0 - pp.x, vy = cy0 - pp.y, dv = Math.hypot(vx, vy) || 1e-6, near = 1 - dv / (REPEL * R);
            if (near > 0) { var push = near * near * 1.6 * p * pp.m; ox += (vx / dv) * push; oy += (vy / dv) * push; grow += 0.35 * near * pp.m; }
          }
          for (var bi = 0; bi < boops.length; bi++) {
            var bo = boops[bi], wx = cx0 - bo.x, wy = cy0 - bo.y, du = Math.hypot(wx, wy) || 1e-6;
            var ag = t - bo.t - du / (3.2 * R);
            if (ag <= 0 || ag > BOOP) continue;
            var amp = 2.4 * p * Math.exp(-4.2 * ag) * Math.sin(15 * ag) * (1 - Math.min(1, du / (2.4 * R)) * 0.5);
            ox += (wx / du) * amp; oy += (wy / du) * amp;
          }
        }
        var sz = Math.min(side * sc * grow, p * 0.98), X = cx0 + ox - sz / 2, Y = cy0 + oy - sz / 2;
        (isPaper ? paperPath : inkPath).push(X, Y, sz);
      }
    }
    ctx.beginPath();
    for (var n = 0; n < inkPath.length; n += 3) ctx.rect(inkPath[n], inkPath[n + 1], inkPath[n + 2], inkPath[n + 2]);
    ctx.fillStyle = ink; ctx.fill();
    ctx.beginPath();
    for (var m = 0; m < paperPath.length; m += 3) ctx.rect(paperPath[m], paperPath[m + 1], paperPath[m + 2], paperPath[m + 2]);
    // feature squares in a mid gray, so they read on the ink squares and on the ground
    ctx.fillStyle = mix(ink, paper, 0.55); ctx.fill();
    return bots;
  }

  /* ------------------------------------------------------------------ frame: pure */

  /** Draws `scene` at time t. No clock, no state: same arguments, same pixels. */
  function frame(ctx, scene, t, o) {
    var dpr = o.dpr || 1, w = o.w, h = o.h, ink = o.ink || '#2A2927', paper = o.paper || '#F3F1EC';
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    // the ground only when the canvas has one; on a transparent canvas o.paper is the color behind it
    if (o.ground !== false) { ctx.fillStyle = paper; ctx.fillRect(0, 0, w, h); }
    var env = { w: w, h: h, pointer: o.pointer, play: o.play !== false, reduced: !!o.reduced, debug: !!(o.debug && o.debug.img), acc: o.acc, heroY: o.heroY, layout: o.layout, hold: o.hold, noTour: !!o.noTour, noLife: !!o.noLife };
    if (scene === 'matrix') { drawMatrix(ctx, t, env, ink, paper); return; }
    if (scene === 'hero-matrix') { drawMatrix(ctx, t, env, ink, paper, heroMatrix(t, env)); return; }
    var bots = scene === 'crowd' ? crowdBots(t, env) : heroBots(t, env);
    if (env.debug) {
      var b = bots[0], im = o.debug.img, fit = PNG_FIT[im.naturalWidth > 300 ? 512 : 200], s = b.R / fit.R;
      ctx.save(); ctx.globalAlpha = 0.5;
      if (im.complete && im.naturalWidth) ctx.drawImage(im, b.x - fit.cx * s, b.y - fit.cy * s, im.naturalWidth * s, im.naturalHeight * s);
      ctx.restore();
      ctx.save(); ctx.globalAlpha = 0.55; drawBot(ctx, b, ink, paper, true); ctx.restore();
      return;
    }
    for (var i = 0; i < bots.length; i++) drawBot(ctx, bots[i], ink, paper, false);
  }

  /** Heads' and eyes' positions (px) at time t, for tests. */
  function debugFrame(scene, t, o) {
    o = o || {};
    var env = { w: o.w || 1600, h: o.h || 560, pointer: null, reduced: false, debug: false, layout: o.layout };
    var bots = scene === 'crowd' ? crowdBots(t, env) : scene === 'matrix' ? matrixBots(t, env) : scene === 'hero-matrix' ? heroMatrix(t, env).bots : heroBots(t, env);
    return bots.map(function (b) {
      var fr = headFrame(b.gaze), eyes = eyePoses(fr);
      return {
        x: b.x, y: b.y, R: b.R, gaze: b.gaze, acc: b.acc,
        eyes: eyes.map(function (e) { return { x: b.x + e.x * b.R, y: b.y + e.y * b.R, depth: e.depth }; })
      };
    });
  }

  /* ------------------------------------------------------------------ mount: the only side effects */

  function mount(canvas, opts) {
    opts = opts || {};
    var scene = opts.scene || canvas.dataset.bot || 'hero';
    var ctx = canvas.getContext('2d');
    var reduced = opts.reduced !== undefined ? opts.reduced : !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
    var t = 0, playing = !reduced, raf = 0, last = 0, onScreen = true, w = 0, h = 0, dpr = 1, drawn = false;
    // opts.pointer === false: idle life only (blinks, drift, the multiply-in); no following, no
    // shying away, no squint on click, no pointer cursor. opts.pointer === 'gaze': the eyes follow
    // the pointer (as the hero's do), nothing more: no shying away, no click, no pointer cursor
    var follow = opts.pointer !== false;
    var gazeOnly = opts.pointer === 'gaze';
    // opts.range: the pointer distance (px) at which the hero's gaze saturates; by default half the window
    var range = opts.range > 0 ? opts.range : 0;
    var pointer = createPointer();
    var isHero = scene === 'hero' || scene === 'hero-matrix'; // both follow the whole window
    // the matrix holds on the page (arrives on view, leaves only a viewport away); data-loop keeps the
    // preview's 7.5 s loop
    // the resting gaze bot (hero, pointer: 'gaze', no tour, e.g. the overlay's notch): event-driven, no frame loop. It
    // draws when the gaze target moves (until the look settles), through each blink of the schedule (one timer to the
    // next), and on resize or a theme change; between those nothing runs. It holds still between events (no drift).
    var idle = gazeOnly && opts.tour === false && scene === 'hero';
    var timer = 0, clockAt = 0, frames = 0;
    var holdMode = scene === 'matrix' && (opts.hold !== undefined ? opts.hold : !('loop' in canvas.dataset));
    var hold = { inAt: reduced ? -100 : Infinity, outAt: null };
    var targets = canvas.dataset.targets ? canvas.dataset.targets.split(',').map(parseFloat) : null;

    function resize() {
      dpr = Math.max(1, window.devicePixelRatio || 1);
      w = canvas.clientWidth; h = canvas.clientHeight;
      var bw = Math.round(w * dpr), bh = Math.round(h * dpr);
      if (canvas.width !== bw || canvas.height !== bh) { canvas.width = bw; canvas.height = bh; }
    }
    function alpha(c) { var m = /rgba\(([^)]+)\)/.exec(c); return m ? parseFloat(m[1].split(/[\s,\/]+/).filter(Boolean)[3]) : c === 'transparent' ? 0 : 1; }
    function behind(el) { // the first opaque background behind a transparent canvas
      for (var e = el.parentElement; e; e = e.parentElement) { var b = getComputedStyle(e).backgroundColor; if (alpha(b) > 0.5) return b; }
      return '#ffffff';
    }
    function layout(cs) {
      var num = function (k) { var v = parseFloat(cs.getPropertyValue(k)); return isFinite(v) ? v : undefined; };
      return { targets: targets, floor: num('--bot-floor'), r: num('--bot-r'), rMax: num('--bot-rmax'), rW: num('--bot-rw') };
    }
    function draw() {
      if (!w || !h) return;
      var cs = getComputedStyle(canvas), bg = cs.backgroundColor, opaque = alpha(bg) > 0.5;
      frames++;
      frame(ctx, scene, t, { w: w, h: h, dpr: dpr, ink: cs.color, paper: opaque ? bg : behind(canvas), ground: opaque, pointer: follow ? pointer : null, play: !gazeOnly, reduced: reduced, debug: opts.debug, acc: opts.acc, heroY: opts.heroY, layout: layout(cs), hold: holdMode ? hold : null, noTour: opts.tour === false, noLife: idle });
      if (!drawn) { drawn = true; if (canvas.parentElement) canvas.parentElement.classList.add('is-live'); }
    }
    function tick(ts) {
      raf = 0;
      if (idle) { advance(); draw(); wake(); return; }
      var dt = last ? Math.min(0.1, (ts - last) / 1000) : 0;
      last = ts;
      t += dt;
      draw();
      schedule();
    }
    // idle mode: the clock runs on the wall clock, read only when something is drawn
    function advance() {
      var now = performance.now();
      if (playing && clockAt) t += Math.max(0, now - clockAt) / 1000;
      clockAt = now;
    }
    // idle mode: a frame per animation frame while the look settles or a blink runs, else one timer to the next blink
    function wake() {
      if (timer) { clearTimeout(timer); timer = 0; }
      if (!playing || !onScreen || document.hidden) return;
      advance();
      var b = blinkNear(t);
      var settling = t < pointer.at + LOOK_MORPH || t < pointer.mAt + pointer.mDur;
      if (settling || b.on) { if (!raf) raf = requestAnimationFrame(tick); return; }
      if (raf) { cancelAnimationFrame(raf); raf = 0; }
      if (b.at !== Infinity) timer = setTimeout(function () { timer = 0; wake(); }, Math.max(0, (b.at - t) * 1000));
    }
    function schedule() {
      if (idle) { wake(); return; }
      if (holdMode && !(hold.inAt <= t + 1e-9)) return; // nothing to animate before the first arrival
      if (!raf && playing && onScreen && !document.hidden) raf = requestAnimationFrame(tick);
    }
    function stop() { if (raf) cancelAnimationFrame(raf); raf = 0; last = 0; if (timer) { clearTimeout(timer); timer = 0; } if (idle) clockAt = 0; }

    var ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(function () { resize(); draw(); }) : null;
    if (ro) ro.observe(canvas);
    var io = typeof IntersectionObserver !== 'undefined' ? new IntersectionObserver(function (es) {
      onScreen = es[es.length - 1].isIntersecting;
      if (onScreen) schedule(); else stop();
    }) : null;
    if (io) io.observe(canvas);
    // hold mode: arrive when 30 % is in view; collapse once more than a viewport away; arrive again
    // when back within that margin (t is frozen while the canvas is off-screen, so the arrival plays
    // when it is seen)
    var ioIn = null, ioNear = null;
    if (holdMode && !reduced && typeof IntersectionObserver !== 'undefined') {
      ioIn = new IntersectionObserver(function (es) {
        if (es[es.length - 1].isIntersecting && !(hold.inAt <= t) ) { hold.inAt = t; hold.outAt = null; schedule(); }
      }, { threshold: 0.3 });
      ioIn.observe(canvas);
      ioNear = new IntersectionObserver(function (es) {
        var near = es[es.length - 1].isIntersecting;
        if (!near && hold.inAt <= t && hold.outAt === null) { hold.outAt = t; }
        else if (near && hold.outAt !== null) { hold.inAt = t; hold.outAt = null; schedule(); }
      }, { rootMargin: '100% 0px' });
      ioNear.observe(canvas);
    }
    function onVis() { if (document.hidden) stop(); else schedule(); }
    document.addEventListener('visibilitychange', onVis);

    // pointer: the hero follows the whole window, the crowd only the canvas
    function look(x, y) { // canvas-relative px
      if (reduced) return;
      var r = canvas.getBoundingClientRect();
      if (range) { pointer.sx = range; pointer.sy = range; }
      else if (isHero) { pointer.sx = window.innerWidth / 2; pointer.sy = window.innerHeight / 2; }
      else { pointer.sx = r.width / 2; pointer.sy = r.height / 2; }
      if (idle) advance();
      setPointer(pointer, x, y, t);
      if (!playing) draw();
      else if (idle) wake();
    }
    function onMove(ev) {
      var r = canvas.getBoundingClientRect();
      look(ev.clientX - r.left, ev.clientY - r.top);
    }
    function onLeave() { if (idle) advance(); releasePointer(pointer, t); if (idle) wake(); }
    function onDown(ev) {
      if (reduced) return;
      var r = canvas.getBoundingClientRect();
      pointer.boops = (pointer.boops || []).filter(function (b) { return t - b.t < 2; });
      pointer.boops.push({ x: ev.clientX - r.left, y: ev.clientY - r.top, t: t });
      onMove(ev);
      if (!raf) schedule();
    }
    if (follow && !gazeOnly && scene === 'hero-matrix') { canvas.addEventListener('pointerdown', onDown); canvas.style.cursor = 'pointer'; canvas.style.touchAction = 'manipulation'; }
    var moveTarget = isHero ? window : canvas;
    if (follow && scene !== 'matrix') {
      moveTarget.addEventListener('pointermove', onMove);
      if (isHero) { document.addEventListener('pointerleave', onLeave); window.addEventListener('blur', onLeave); }
      else canvas.addEventListener('pointerleave', onLeave);
    }

    resize(); draw(); schedule();

    var api = {
      play: function () { if (reduced) return; playing = true; clockAt = 0; schedule(); },
      pause: function () { if (idle) advance(); playing = false; stop(); },
      seek: function (nt) { t = Math.max(0, nt); if (idle && clockAt) clockAt = performance.now(); draw(); },
      // the eyes look at (x, y), canvas-relative px: for a host that sees the pointer beyond its own window
      look: function (x, y) { if (follow) look(x, y); },
      destroy: function () {
        stop();
        if (ro) ro.disconnect();
        if (io) io.disconnect();
        if (ioIn) ioIn.disconnect();
        if (ioNear) ioNear.disconnect();
        document.removeEventListener('visibilitychange', onVis);
        moveTarget.removeEventListener('pointermove', onMove);
        document.removeEventListener('pointerleave', onLeave);
        window.removeEventListener('blur', onLeave);
        canvas.removeEventListener('pointerleave', onLeave);
        canvas.removeEventListener('pointerdown', onDown);
      },
      get t() { return t; },
      get playing() { return playing; },
      // test hooks: frames drawn so far, whether a frame is requested, whether a timer waits for the next blink (idle mode)
      get frames() { return frames; },
      get looping() { return raf !== 0; },
      get waiting() { return timer !== 0; },
      get nextBlinkIn() { var tn = t + (playing && clockAt ? (performance.now() - clockAt) / 1000 : 0), b = blinkNear(tn); return b.on ? 0 : b.at - tn; }
    };
    return api;
  }

  function mountAll() {
    var list = document.querySelectorAll('canvas[data-bot]'), out = [];
    for (var i = 0; i < list.length; i++) {
      var c = list[i];
      if (c.__modeBot) { out.push(c.__modeBot); continue; }
      c.__modeBot = mount(c, { scene: c.dataset.bot });
      out.push(c.__modeBot);
    }
    return out;
  }

  window.ModeBot = {
    frame: frame,
    mount: mount,
    mountAll: mountAll,
    debugFrame: debugFrame,
    createPointer: createPointer,
    setPointer: setPointer,
    releasePointer: releasePointer,
    lookTarget: lookTarget,
    TOUR_TIME: TOUR_TIME,
    MATRIX_PERIOD: MATRIX_PERIOD
  };

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mountAll);
  else mountAll();
})();
