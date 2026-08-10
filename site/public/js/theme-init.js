/* Runs synchronously in <head>: pick the theme before first paint so night
   visitors never see a flash of day. main.js takes it from here. */
(function () {
  try {
    var t = localStorage.getItem('omk-theme');
    if (t !== 'day' && t !== 'night') {
      t = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'night' : 'day';
    }
    document.documentElement.setAttribute('data-theme', t);
  } catch (e) { /* storage blocked → keep the default day theme */ }
})();
