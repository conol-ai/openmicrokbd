/* ============================================================
   OPEN MICRO KBD — page behaviour
   Vanilla JS, no deps.
   Modules below are IIFE-scoped and independent of each other.
   ============================================================ */
(function () {
  'use strict';

  var reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var $  = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var clamp = function (v, a, b) { return v < a ? a : v > b ? b : v; };

  /* ------------------------------------------------------------
     1. THEME (day / night) — persisted, crossfaded via CSS
     theme-init.js already set data-theme before first paint.
     ------------------------------------------------------------ */
  (function theme() {
    var KEY = 'omk-theme';
    var root = document.documentElement;
    var btn = $('#themeToggle');
    var label = $('#themeLabel');

    apply(root.getAttribute('data-theme') === 'night' ? 'night' : 'day', true);

    function apply(mode, silent) {
      root.setAttribute('data-theme', mode);
      if (label) label.textContent = mode === 'night' ? 'NIGHT' : 'DAY';
      if (btn) btn.setAttribute('aria-pressed', mode === 'night' ? 'true' : 'false');
      // keep the browser chrome colour in step with a manual toggle
      $$('meta[name="theme-color"]').forEach(function (m) {
        m.setAttribute('content', mode === 'night' ? '#131a3a' : '#f2e2bd');
      });
      if (!silent) {
        try { localStorage.setItem(KEY, mode); } catch (e) {}
      }
    }

    if (btn) {
      btn.addEventListener('click', function () {
        apply(root.getAttribute('data-theme') === 'night' ? 'day' : 'night');
        window.OMK_blip && window.OMK_blip(520, 0.05);
      });
    }
  })();

  /* ------------------------------------------------------------
     2. STARS (night sky) — generated once, twinkle via CSS
     ------------------------------------------------------------ */
  (function stars() {
    var host = $('#stars');
    if (!host) return;
    var n = window.innerWidth < 700 ? 34 : 64;
    var frag = document.createDocumentFragment();
    for (var i = 0; i < n; i++) {
      var s = document.createElement('span');
      s.className = 'star' + (i % 9 === 0 ? ' star--big' : '');
      s.style.left = (Math.random() * 100).toFixed(2) + '%';
      s.style.top = (Math.random() * 62).toFixed(2) + '%';
      s.style.animationDelay = (Math.random() * 3.2).toFixed(2) + 's';
      frag.appendChild(s);
    }
    host.appendChild(frag);
  })();

  /* ------------------------------------------------------------
     3. NAV: mobile burger + hide-on-scroll-down
     ------------------------------------------------------------ */
  (function nav() {
    var burger = $('#navBurger');
    var links = $('.nav__links');
    var navEl = $('#nav');

    if (burger && links) {
      burger.addEventListener('click', function () {
        var open = links.classList.toggle('is-open');
        burger.setAttribute('aria-expanded', open ? 'true' : 'false');
      });
      links.addEventListener('click', function (e) {
        if (e.target.tagName === 'A') {
          links.classList.remove('is-open');
          burger.setAttribute('aria-expanded', 'false');
        }
      });
    }

    var last = 0;
    window.addEventListener('scroll', function () {
      var y = window.pageYOffset;
      if (navEl && !reduceMotion) {
        var hide = y > last && y > 220 && !(links && links.classList.contains('is-open'));
        navEl.style.transform = hide ? 'translateY(-140%)' : 'translateY(0)';
      }
      last = y;
    }, { passive: true });

    // keyboard users tabbing into the hidden nav must get it back on screen
    if (navEl) {
      navEl.addEventListener('focusin', function () {
        navEl.style.transform = 'translateY(0)';
      });
    }
  })();

  /* ------------------------------------------------------------
     4. SCROLL PROGRESS BAR + hero/cloud parallax
     Single rAF-throttled scroll handler for both.
     ------------------------------------------------------------ */
  (function scrollFx() {
    var fill = $('#progressFill');
    var layers = $$('[data-parallax]');
    var heroArt = $('#heroArt');
    var ticking = false;

    function frame() {
      ticking = false;
      var y = window.pageYOffset;
      var max = document.documentElement.scrollHeight - window.innerHeight;
      if (fill) fill.style.width = (max > 0 ? clamp(y / max, 0, 1) * 100 : 0).toFixed(2) + '%';

      if (reduceMotion) return;

      // clouds drift up/down at their own rate
      for (var i = 0; i < layers.length; i++) {
        var r = parseFloat(layers[i].getAttribute('data-parallax')) || 0;
        layers[i].style.transform = 'translate3d(0,' + (y * r).toFixed(1) + 'px,0)';
      }
      // hero illustration lags behind the copy
      if (heroArt && y < window.innerHeight * 1.4) {
        heroArt.style.setProperty('--pmy-scroll', (y * -0.06).toFixed(1) + 'px');
      }
    }

    window.addEventListener('scroll', function () {
      if (!ticking) { ticking = true; requestAnimationFrame(frame); }
    }, { passive: true });
    frame();
  })();

  /* ------------------------------------------------------------
     5. POINTER PARALLAX on the hero illustration
     ------------------------------------------------------------ */
  (function pointerParallax() {
    if (reduceMotion) return;
    var art = $('#heroArt');
    var hero = $('#hero');
    if (!art || !hero || window.matchMedia('(hover: none)').matches) return;

    hero.addEventListener('pointermove', function (e) {
      var r = hero.getBoundingClientRect();
      var nx = (e.clientX - r.left) / r.width - 0.5;   // -0.5 … 0.5
      var ny = (e.clientY - r.top) / r.height - 0.5;
      art.style.setProperty('--pmx', (nx * 26).toFixed(1) + 'px');
      // the CSS sums this tilt with the scroll offset, so neither write clobbers the other
      art.style.setProperty('--pmy-tilt', (ny * 16).toFixed(1) + 'px');
    });
    hero.addEventListener('pointerleave', function () {
      art.style.setProperty('--pmx', '0px');
      art.style.setProperty('--pmy-tilt', '0px');
    });
  })();

  /* ------------------------------------------------------------
     6. SCROLL REVEALS (IntersectionObserver + stagger)
     ------------------------------------------------------------ */
  (function reveals() {
    var items = $$('.reveal');
    if (!items.length) return;

    if (reduceMotion || !('IntersectionObserver' in window)) {
      items.forEach(function (el) { el.classList.add('revealed'); });
      return;
    }

    // Children of [data-stagger] get an incremental transition-delay.
    $$('[data-stagger]').forEach(function (group) {
      $$('.reveal', group).forEach(function (child, i) {
        child.style.transitionDelay = (i * 90) + 'ms';
      });
    });

    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.classList.add('revealed');
          io.unobserve(en.target);
        }
      });
    }, { threshold: 0.12, rootMargin: '0px 0px -8% 0px' });

    items.forEach(function (el) { io.observe(el); });
  })();

  /* ------------------------------------------------------------
     7. WEBAUDIO BLIP (square wave) + mute toggle
     Context is created lazily on the first real user gesture,
     so nothing ever autoplays.
     ------------------------------------------------------------ */
  var audio = (function () {
    var ctx = null;
    var on = true;
    var Ctor = window.AudioContext || window.webkitAudioContext;

    function ensure() {
      if (!Ctor) return null;
      if (!ctx) { try { ctx = new Ctor(); } catch (e) { return null; } }
      if (ctx.state === 'suspended') ctx.resume();
      return ctx;
    }

    function blip(freq, dur, type) {
      if (!on) return;
      var c = ensure();
      if (!c) return;
      var t = c.currentTime;
      var osc = c.createOscillator();
      var g = c.createGain();
      osc.type = type || 'square';
      osc.frequency.setValueAtTime(freq || 660, t);
      // short pluck envelope, kept quiet on purpose
      g.gain.setValueAtTime(0.0001, t);
      g.gain.linearRampToValueAtTime(0.055, t + 0.006);
      g.gain.exponentialRampToValueAtTime(0.0001, t + (dur || 0.07));
      osc.connect(g).connect(c.destination);
      osc.start(t);
      osc.stop(t + (dur || 0.07) + 0.02);
    }

    var btn = $('#soundToggle');
    var label = $('#soundLabel');
    if (btn) {
      btn.addEventListener('click', function () {
        on = !on;
        btn.setAttribute('aria-pressed', on ? 'true' : 'false');
        if (label) label.textContent = 'SOUND: ' + (on ? 'ON' : 'OFF');
        if (on) blip(760, 0.06);
      });
    }

    window.OMK_blip = blip;
    return { blip: blip, isOn: function () { return on; } };
  })();

  /* ------------------------------------------------------------
     8. THE INTERACTIVE MACRO PAD — real layout:
        knob · key · key · joystick
        key  · key · key · key
        key  · key · key · key
        touch · 2U key · key
     ------------------------------------------------------------ */
  (function pad() {
    var padEl = $('#pad');
    if (!padEl) return;

    var keys = $$('.key', padEl);
    var scenes = $$('.scene');
    var screenTitle = $('#screenTitle');
    var lastKeyLog = $('#lastKey');
    var timers = [];

    function clearTimers() {
      timers.forEach(clearTimeout);
      timers = [];
    }
    function later(fn, ms) { timers.push(setTimeout(fn, ms)); }

    function showScene(name) {
      scenes.forEach(function (s) {
        s.classList.toggle('is-active', s.getAttribute('data-scene') === name);
      });
    }

    var TITLES = {
      idle:    'OPEN MICRO KBD · READY',
      voice:   'VOICE KEY · REC',
      knob:    'ENCODER · EFFORT',
      joy:     'THE STICK · ANALOG',
      touch:   'TOUCH PAD · TAP',
      multi:   'TASK SWITCHER',
      perm:    'PERMISSION REQUEST',
      macro:   'MACRO · SHIP IT',
      profile: 'PROFILE MANAGER',
      blank:   'EMPTY SLOT',
      happy:   ':)'
    };

    /* ---- voice scene ---- */
    var WAVE_BARS = 22;
    var waveHost = $('#wave');
    if (waveHost) {
      for (var i = 0; i < WAVE_BARS; i++) {
        var b = document.createElement('i');
        b.style.animationDelay = (i * 45) + 'ms';
        b.style.animationDuration = (420 + (i % 5) * 90) + 'ms';
        waveHost.appendChild(b);
      }
    }
    var TRANSCRIPT = 'refactor the auth service to use the new token store, then run the tests and tell me what broke';
    function runVoice() {
      var out = $('#transcript');
      if (!out) return;
      out.textContent = '';
      if (reduceMotion) { out.textContent = TRANSCRIPT; return; }
      var n = 0;
      (function type() {
        out.textContent = TRANSCRIPT.slice(0, n);
        n += 2;
        if (n <= TRANSCRIPT.length + 2) later(type, 26);
      })();
    }

    /* ---- knob / effort scene ---- */
    var knob = $('#knob');
    var knobDial = $('#knobDial');
    var effortFill = $('#effortFill');
    var effortThumb = $('#effortThumb');
    var effortValue = $('#effortValue');
    var effortNote = $('#effortNote');
    var effort = 45;                 // 0..100 — BALANCED, matching the HTML defaults
    var rotation = (45 - 50) * 2.7;  // degrees, purely visual; 50% points at 12 o'clock

    var LEVELS = [
      [0,   'INSTANT',  'Snap answers. No deliberation, no detours.'],
      [20,  'FAST',     'Quick passes — good for edits and small fixes.'],
      [45,  'BALANCED', 'The daily driver. Thinks, but keeps moving.'],
      [70,  'THOROUGH', 'Reads more, checks itself, costs a little time.'],
      [88,  'DEEP',     'Long chains of reasoning for gnarly problems.']
    ];

    function setEffort(v, spin, keepNote) {
      effort = clamp(v, 0, 100);
      if (effortFill) effortFill.style.width = effort + '%';
      if (effortThumb) effortThumb.style.left = effort + '%';

      var lvl = LEVELS[0];
      for (var i = 0; i < LEVELS.length; i++) if (effort >= LEVELS[i][0]) lvl = LEVELS[i];
      if (effortValue) effortValue.textContent = lvl[1] + '  ·  ' + Math.round(effort) + '%';
      // keepNote preserves the authored "volume wheel" hint until the knob actually turns
      if (effortNote && !keepNote) effortNote.textContent = lvl[2];

      if (knob) {
        knob.setAttribute('aria-valuenow', Math.round(effort));
        knob.setAttribute('aria-valuetext', lvl[1]);
      }
      if (spin !== false && knobDial) {
        rotation = (effort - 50) * 2.7;
        knobDial.style.transform = 'rotate(' + rotation.toFixed(1) + 'deg)';
      }
    }

    function nudgeEffort(delta) {
      showScene('knob');
      if (screenTitle) screenTitle.textContent = TITLES.knob;
      setEffort(effort + delta);
      audio.blip(360 + effort * 6, 0.035, 'square');
    }

    if (knob) {
      // drag (pointer) — vertical or horizontal, whichever moves more
      var dragging = false, lastY = 0, lastX = 0;
      knob.addEventListener('pointerdown', function (e) {
        dragging = true; lastY = e.clientY; lastX = e.clientX;
        try { knob.setPointerCapture(e.pointerId); } catch (err) {}
        showScene('knob');
        if (screenTitle) screenTitle.textContent = TITLES.knob;
        e.preventDefault();
      });
      knob.addEventListener('pointermove', function (e) {
        if (!dragging) return;
        var dy = lastY - e.clientY;
        var dx = e.clientX - lastX;
        var d = Math.abs(dy) > Math.abs(dx) ? dy : dx;
        if (Math.abs(d) < 1) return;
        lastY = e.clientY; lastX = e.clientX;
        var before = Math.round(effort / 5);
        setEffort(effort + d * 0.6);
        if (Math.round(effort / 5) !== before) audio.blip(340 + effort * 6, 0.028);
      });
      ['pointerup', 'pointercancel'].forEach(function (ev) {
        knob.addEventListener(ev, function () { dragging = false; });
      });
      // wheel
      knob.addEventListener('wheel', function (e) {
        e.preventDefault();
        nudgeEffort(e.deltaY < 0 ? 5 : -5);
      }, { passive: false });
      // keyboard
      knob.addEventListener('keydown', function (e) {
        var map = { ArrowRight: 5, ArrowUp: 5, ArrowLeft: -5, ArrowDown: -5, Home: -100, End: 100 };
        if (e.key in map) { e.preventDefault(); nudgeEffort(map[e.key]); }
      });
    }
    setEffort(effort, true, true);

    /* ---- joystick ---- */
    var joyEl = $('#joy');
    var joyField = $('#joyField');
    var joyDot = $('#joyDot');
    var joyLabel = $('#joyModeLabel');
    var joyNote = $('#joyNote');
    var joyModeEls = $$('#joyModes .pbadge');
    var JOY_TRAVEL = 14;   // nub travel in px; the on-screen dot exaggerates it
    var JOY_MODES = [
      ['MOUSE MODE',  'Proportional pointer glide — the push switch is left click. Press the stick to cycle modes.'],
      ['GRADE MODE',  'DaVinci Resolve color wheels by feel — the drag engages while deflected.'],
      ['ARROW MODE',  'Four directions and a press — five slots, mapped like any other keys.']
    ];
    var joyIdx = 0;

    function setJoyMode(i) {
      joyIdx = ((i % JOY_MODES.length) + JOY_MODES.length) % JOY_MODES.length;
      joyModeEls.forEach(function (b, k) { b.classList.toggle('is-on', k === joyIdx); });
      if (joyLabel) joyLabel.textContent = JOY_MODES[joyIdx][0];
      if (joyNote) joyNote.textContent = JOY_MODES[joyIdx][1];
    }

    function joyScene() {
      clearTimers();
      keys.forEach(function (k) { k.classList.remove('is-active'); });
      showScene('joy');
      if (screenTitle) screenTitle.textContent = TITLES.joy;
    }

    function setDeflect(dx, dy) {
      if (!joyEl) return;
      joyEl.style.setProperty('--jx', dx.toFixed(1) + 'px');
      joyEl.style.setProperty('--jy', dy.toFixed(1) + 'px');
      if (joyDot) {
        joyDot.style.setProperty('--dx', clamp(dx * 5, -170, 170).toFixed(1) + 'px');
        joyDot.style.setProperty('--dy', clamp(dy * 4, -52, 52).toFixed(1) + 'px');
      }
    }

    if (joyEl) {
      var jDragging = false, jMoved = false, jx0 = 0, jy0 = 0;
      joyEl.addEventListener('pointerdown', function (e) {
        jDragging = true; jMoved = false; jx0 = e.clientX; jy0 = e.clientY;
        joyEl.classList.add('is-drag');
        if (joyField) joyField.classList.add('is-live');
        try { joyEl.setPointerCapture(e.pointerId); } catch (err) {}
        joyScene();
        audio.blip(520, 0.045);
        e.preventDefault();
      });
      joyEl.addEventListener('pointermove', function (e) {
        if (!jDragging) return;
        var dx = e.clientX - jx0;
        var dy = e.clientY - jy0;
        if (Math.abs(dx) + Math.abs(dy) > 4) jMoved = true;
        var m = Math.sqrt(dx * dx + dy * dy) || 1;
        var r = Math.min(m, JOY_TRAVEL);
        setDeflect(dx / m * r, dy / m * r);
      });
      ['pointerup', 'pointercancel'].forEach(function (ev) {
        joyEl.addEventListener(ev, function () {
          if (!jDragging) return;
          jDragging = false;
          joyEl.classList.remove('is-drag');
          if (joyField) joyField.classList.remove('is-live');
          setDeflect(0, 0);
          if (!jMoved) {
            setJoyMode(joyIdx + 1);
            audio.blip(560 + joyIdx * 90, 0.06);
            if (lastKeyLog) lastKeyLog.textContent = 'STICK PRESS → ' + JOY_MODES[joyIdx][0];
          } else {
            if (lastKeyLog) lastKeyLog.textContent = 'STICK → GLIDE';
          }
        });
      });
      joyEl.addEventListener('keydown', function (e) {
        var K = { ArrowLeft: [-JOY_TRAVEL, 0], ArrowRight: [JOY_TRAVEL, 0], ArrowUp: [0, -JOY_TRAVEL], ArrowDown: [0, JOY_TRAVEL] };
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          joyScene();
          setJoyMode(joyIdx + 1);
          audio.blip(600, 0.05);
          return;
        }
        if (K[e.key]) {
          e.preventDefault();
          joyScene();
          setDeflect(K[e.key][0], K[e.key][1]);
          later(function () { setDeflect(0, 0); }, 220);
        }
      });
      setJoyMode(0);
    }

    /* ---- touch pad (tap only — the single-zone pad cannot detect swipes) ---- */
    var touchEl = $('#touchPad');

    function padTap() {
      clearTimers();
      keys.forEach(function (k) { k.classList.remove('is-active'); });
      showScene('touch');
      if (screenTitle) screenTitle.textContent = TITLES.touch;
      if (touchEl) {
        touchEl.classList.remove('is-tap');
        void touchEl.offsetWidth;   /* restart the ripple animation */
        touchEl.classList.add('is-tap');
      }
      audio.blip(880, 0.05, 'triangle');
      if (lastKeyLog) lastKeyLog.textContent = 'PAD → TAP';
    }

    if (touchEl) {
      touchEl.addEventListener('pointerdown', function (e) { e.preventDefault(); });
      touchEl.addEventListener('pointerup', function () { padTap(); });
      touchEl.addEventListener('keydown', function (e) {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); padTap(); }
      });
    }

    /* ---- multi-task scene ---- */
    var taskEls = $$('#tasks .task');
    var taskIdx = 0;
    function cycleTasks() {
      taskEls.forEach(function (t, i) { t.classList.toggle('is-focus', i === taskIdx); });
      var states = ['RUNNING', 'RUNNING', 'QUEUED'];
      taskEls.forEach(function (t, i) {
        var s = $('.task__state', t);
        if (s) s.textContent = i === taskIdx ? 'FOCUSED' : states[i];
      });
      taskIdx = (taskIdx + 1) % taskEls.length;
    }

    /* ---- permission scene ---- */
    var permResult = $('#permResult');
    $$('[data-perm]').forEach(function (b) {
      b.addEventListener('click', function () {
        var allow = b.getAttribute('data-perm') === 'allow';
        if (permResult) {
          permResult.textContent = allow
            ? '> Allowed. Agent resumed — deploying build.'
            : '> Denied. Agent paused and asked for another plan.';
        }
        audio.blip(allow ? 880 : 220, 0.09, allow ? 'square' : 'sawtooth');
      });
    });
    function resetPerm() {
      if (permResult) permResult.textContent = 'One press brings up exactly what needs your attention.';
      var p = $('#popup');
      if (p && !reduceMotion) { p.style.animation = 'none'; void p.offsetWidth; p.style.animation = ''; }
    }

    /* ---- macro scene ---- */
    var comboItems = $$('#combo li');
    function runMacro() {
      comboItems.forEach(function (li) { li.classList.remove('is-in'); });
      comboItems.forEach(function (li, i) {
        later(function () {
          li.classList.add('is-in');
          audio.blip(520 + i * 110, 0.04);
        }, reduceMotion ? 0 : 110 + i * 130);
      });
    }

    /* ---- profile scene ---- */
    var pbadges = $$('#profiles .pbadge');
    var profileNames = ['CODE', 'EDIT', 'COLOR', 'GAME'];
    var profileNotes = [
      '13 keys mapped for agents, git and the terminal.',
      'Timeline scrub on the knob. Cut, ripple, export.',
      'Lift / gamma / gain on the stick. Grade by feel.',
      'Macros, comms, and a knob for master volume.'
    ];
    var profileIdx = -1;
    function cycleProfile() {
      profileIdx = (profileIdx + 1) % profileNames.length;
      pbadges.forEach(function (b, i) { b.classList.toggle('is-on', i === profileIdx); });
      var n = $('#profileName'); if (n) n.textContent = profileNames[profileIdx];
      var note = $('#profileNote'); if (note) note.textContent = profileNotes[profileIdx];
    }

    /* ---- dispatch ---- */
    var SCENE_SETUP = {
      voice:   runVoice,
      knob:    function () { setEffort(effort, false, true); },
      multi:   cycleTasks,
      perm:    resetPerm,
      macro:   runMacro,
      profile: cycleProfile
    };
    var TONES = { voice: 700, knob: 480, multi: 600, perm: 400, macro: 820, profile: 560, blank: 300, happy: 990 };

    function activate(key) {
      var fn = key.getAttribute('data-fn') || 'blank';

      clearTimers();
      keys.forEach(function (k) { k.classList.toggle('is-active', k === key); });

      showScene(fn);
      if (screenTitle) screenTitle.textContent = TITLES[fn] || TITLES.idle;

      if (fn === 'blank') {
        var lbl = $('#blankLabel');
        if (lbl) lbl.textContent = key.getAttribute('data-blankname') || 'UNASSIGNED SLOT';
      }
      if (SCENE_SETUP[fn]) SCENE_SETUP[fn]();

      var bind = (key.getAttribute('data-bind') || '?').toUpperCase();
      var name = ($('.key__lbl', key) || {}).textContent || fn.toUpperCase();
      if (lastKeyLog) lastKeyLog.textContent = 'KEY [' + bind + '] → ' + name;

      audio.blip(TONES[fn] || 660, 0.07);
    }

    function press(key) {
      key.classList.add('is-down');
      setTimeout(function () { key.classList.remove('is-down'); }, 110);
      activate(key);
    }

    keys.forEach(function (key) {
      key.addEventListener('click', function () { press(key); });
    });

    // physical keyboard bindings (1-6 / Q W E R T Y U)
    var byBind = {};
    keys.forEach(function (k) {
      var b = (k.getAttribute('data-bind') || '').toLowerCase();
      if (b) byBind[b] = k;
    });
    var held = {};
    document.addEventListener('keydown', function (e) {
      if (e.metaKey || e.ctrlKey || e.altKey || e.repeat) return;
      var tag = (e.target.tagName || '').toLowerCase();
      if (tag === 'input' || tag === 'textarea' || e.target.isContentEditable) return;

      // track keys case-insensitively so a Shift released mid-press can't wedge them
      var bind = (e.key || '').toLowerCase();
      var k = byBind[bind];
      if (!k || held[bind]) return;

      // only hijack physical keys while the demo is on screen
      var r = padEl.getBoundingClientRect();
      if (r.bottom < 0 || r.top > window.innerHeight) return;

      held[bind] = true;
      e.preventDefault();
      k.classList.add('is-down');
      activate(k);
    });
    document.addEventListener('keyup', function (e) {
      var bind = (e.key || '').toLowerCase();
      var k = byBind[bind];
      if (k) k.classList.remove('is-down');
      delete held[bind];
    });
  })();

  /* ------------------------------------------------------------
     9. Button press feedback
     ------------------------------------------------------------ */
  (function buttons() {
    $$('.btn').forEach(function (b) {
      b.addEventListener('click', function () {
        window.OMK_blip && window.OMK_blip(700, 0.06);
      });
    });
    // Nav/footer anchors also get a tiny confirmation blip.
    $$('.nav__links a, .footer__links a, .scroll-hint').forEach(function (a) {
      a.addEventListener('click', function () {
        window.OMK_blip && window.OMK_blip(600, 0.045);
      });
    });
  })();

})();
