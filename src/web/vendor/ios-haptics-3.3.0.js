/**
* ios-haptics v3.3.0
* tijn.dev
* @license MIT
**/
function e(){return typeof navigator>`u`?!1:/iPad|iPhone|iPod/.test(navigator.userAgent)||navigator.platform===`MacIntel`&&navigator.maxTouchPoints>1}function t(t){if(!t||typeof window>`u`||!e()||t.querySelector(`:scope > [data-haptic-trigger]`))return;let n=document.createElement(`label`);n.setAttribute(`data-haptic-trigger`,``),n.setAttribute(`aria-hidden`,`true`),Object.assign(n.style,{position:`absolute`,inset:`0`,touchAction:`manipulation`}),n.style.setProperty(`-webkit-tap-highlight-color`,`transparent`);let r=document.createElement(`input`);r.type=`checkbox`,r.setAttribute(`switch`,``),Object.assign(r.style,{position:`absolute`,width:`1px`,height:`1px`,margin:`0`,visibility:`hidden`}),r.addEventListener(`click`,e=>e.stopPropagation()),n.append(r),getComputedStyle(t).position===`static`&&(t.style.position=`relative`),t.insertAdjacentElement(`beforeend`,n)}export{t as hapticTrigger};