#version 300 es
precision highp float;
in vec2 uv;
out vec4 pixel;
uniform sampler2D trail;
uniform vec2 size;
uniform vec4 signal;
uniform vec2 dynamics;
uniform float wave[48];
uniform float clock;
uniform float drive;
uniform float delta;
uniform float blend;
uniform float colorMode;
uniform int scene;
uniform int previous;

const float PI = 3.14159265;
const vec3 silver = vec3(0.70, 0.85, 0.96);
const vec3 shadow = vec3(0.26, 0.39, 0.57);
mat2 rotate(float angle) { float c = cos(angle), s = sin(angle); return mat2(c, s, -s, c); }
float thread(float distance, float width) { return exp(-abs(distance) / width); }
float scope(float x) {
  float slot = clamp(x, 0., .999) * 47.;
  int i = int(slot);
  return mix(wave[i], wave[i + 1], fract(slot));
}
vec3 palette(vec3 a, vec3 b, float weight) { return mix(mix(shadow, silver, weight), mix(a, b, weight), colorMode); }

vec3 ink(vec2 p, float time) {
  p = rotate(-.34 + .13 * sin(time * .39)) * p;
  p /= 1. + signal.x * .34;
  p.x *= .73;
  p.x += dynamics.y * .12;
  float x = p.x, y = p.y;
  float sweep = .29 * sin(2.8*x + time*.51) + .096 * sin(6.*x - time*.85) + .045 * scope(x*.52 + .5);
  float fold = .18 + .17*signal.y;
  float lines = 0.;
  for (int i = 0; i < 6; i++) {
    float k = float(i) - 2.5;
    float filament = sweep + k*fold*.24*sin(3.8*x + .7*k + time*.56) + .033*k*sin(7.4*x - time*.63);
    lines += thread(y - filament, .0045 + .003 * signal.z) * (.65 + .35*cos(x*4. - k + time*.38));
  }
  float smoke = thread(y - sweep, .105 + .069*signal.x) * (.24 + .10 * sin(x*11. + time));
  float wisps = thread(y - sweep*.78 + .15*sin(x*4. + time*.43), .033) * .12;
  float mask = exp(-1.48 * x*x - 2.1 * y*y) * (.78 + .20 * sin(x*2. - time*.33));
  return mask * palette(vec3(.13,.24,.71), vec3(.82,.72,.99), .70) * (lines*.34 + smoke + wisps);
}

vec3 vortex(vec2 p, float time) {
  p *= rotate(.09*time);
  p -= vec2(.12 * sin(time*.62), .05 * cos(time*.71));
  float r = length(p);
  float angle = atan(p.y, p.x);
  float hole = .065 + signal.x * .083;
  float depth = log(max(r,.008) + .07);
  float segments = 6.*angle + 4.8*depth + .30*sin(3.*angle - time);
  float ribs = pow(.5+.5*cos(segments), 5.);
  float rings = pow(.5+.5*cos(13.*depth + 1.3*time + .8*sin(segments*.5)), 13.);
  float shadowing = smoothstep(hole*.34, hole+.06, r) * exp(-1.35*r);
  return shadowing * (ribs*.24 + rings*.62) * palette(vec3(.06,.34,.59), vec3(.27,.93,.84), .38+.48*rings);
}

vec3 silk(vec2 p, float time) {
  p = rotate(.21) * p;
  vec3 sum = vec3(0.);
  for (int i = 0; i < 3; i++) {
    float k = float(i);
    float crest = .20 * sin(p.x*2.7 + k*1.8 + .5*time) + .09 * cos(p.x*5.6 - .8*time + k) + (k-1.)*.112;
    float thickness = .016 + (.045 + .062*signal.y)*(.5+.5*sin(2.8*p.x + k + .45*time));
    float surf = (p.y - crest)/thickness;
    float cloth = exp(-surf*surf*.62);
    float detail = (.12+.40*signal.z) * pow(.5+.5*cos(surf*22. + 7.*p.x), 7.);
    float edge = thread(surf-.6, .12) * .24;
    vec3 tint = palette(vec3(.18,.24,.68), vec3(.97,.53,.46), .3 + k*.32);
    sum += tint * (cloth*(.16+detail) + edge) * exp(-1.1*p.x*p.x);
  }
  return sum;
}

vec3 interference(vec2 p, float time) {
  vec2 center = vec2(.26 + .12*signal.x, .10*sin(time*.54));
  float a = length(p-center), b = length(p+center);
  float spread = 22. + signal.y * 8.;
  float f = cos(a*spread - time*2.9), g = cos(b*spread - time*2.9 + .7);
  float beat = sin((a+b)*13. - time*2.) * dynamics.x;
  float loops = pow(abs((f+g)*.5), 7.);
  float mesh = pow(.5+.5*f*g, 13.) * .18;
  return exp(-1.4*dot(p,p)) * (loops * (.20 + .16*beat) + mesh) *
    palette(vec3(.27,.12,.51), vec3(.80,.54,.94), .52+.35*f*g);
}

vec3 orbit(vec2 p, float time) {
  vec3 sum = vec3(0.);
  for (int i = 0; i < 4; i++) {
    float k = float(i);
    vec2 q = rotate(.72*k + .11*time + .30*dynamics.y)*p;
    float radius = .30 + k*.145 + .086*signal.x;
    q.y /= .33+.08*k;
    float theta = atan(q.y,q.x);
    float radial = length(q)-radius;
    float dots = pow(.5+.5*cos(theta*(13.+k*4.) + time*(1.6+k*.43)), 15.);
    float arc = .15 + .85*pow(.5+.5*sin(theta - time*.31 + k), 3.);
    float light = (thread(radial,.007 + .005*signal.z) + .12*thread(radial,.045)) * arc;
    sum += palette(vec3(.16,.42,.61), vec3(.96,.73,.35), .25 + .17*k) * light * (.29 + dots*.66);
  }
  return sum;
}

vec3 prism(vec2 p, float time) {
  float radius = length(p);
  float theta = atan(p.y,p.x) + .13*time + .13*sin(radius*6. + time*.4);
  float wedge = abs(mod(theta + PI/8.,PI/4.) - PI/8.);
  vec2 q = vec2(cos(wedge), sin(wedge))*radius;
  q.x += .034*cos(time*.71 + q.y*9.) + .04*signal.x;
  float tile = abs(fract(q.x*5.-.38)-.5) + abs(fract(q.y*8.+q.x*.84)-.5);
  float bevel = thread(tile-(.54 + .057*sin(time*.71)), .036);
  float facet = smoothstep(.28,.31,tile)*(1.-smoothstep(.60,.63,tile));
  float fade = exp(-1.85*radius*radius)*smoothstep(.034,.20,radius);
  float weight = .48+.36*cos(q.x*19. - q.y*7. - time*.53);
  return (bevel*(.41+.26*signal.z)+facet*.13)*fade * palette(vec3(.26,.12,.69), vec3(.15,.89,.78),weight);
}

vec3 rain(vec2 p, float time) {
  p.y *= 1.6;
  p.x *= 1. + .30*p.y;
  float marks = 0.;
  for (int i = 0; i < 5; i++) {
    float k = float(i);
    float phase = time*.48 + k*.24;
    float epoch = floor(phase) + 2.7*k;
    vec2 center = vec2(sin(epoch*8.31)*1.02, .52*cos(epoch*12.72));
    float r = length(p - center);
    float age = fract(phase);
    float ripple = r - age * (.46 + .18*signal.x);
    float wavefront = thread(ripple,.011) + thread(ripple+.048,.008)*.38;
    marks += wavefront * sin(age*PI) * exp(-3.8*age);
  }
  float current = .07 * pow(.5+.5*cos(p.y*32. - time*1.6 + .4*cos(p.x*7.)), 7.);
  return exp(-.92*dot(p,p))*(marks*(.68+.40*dynamics.x)+current) * palette(vec3(.13,.32,.64), vec3(.33,.78,.98),.67);
}

vec3 aurora(vec2 p, float time) {
  vec3 sum = vec3(0.);
  float drift = p.x + .07*sin(time*.34) + .06*dynamics.y;
  for (int i = 0; i < 3; i++) {
    float k = float(i);
    float base = -.05 + .18*sin(2.3*drift - time*.33 + k*.9) - .11*k + .08*sin(5.2*drift + k + time*.42);
    float y = p.y - base;
    float rib = .55+.45*sin(28.*drift + 3.*sin(drift*8. + time*.53) + k*2.);
    float curtain = smoothstep(-.020,.008,y)*exp(-y / (.125 + signal.x*.081));
    float glow = thread(y,.039) * (.12 + .25*signal.y);
    float fade = exp(-1.4*drift*drift) * smoothstep(-1.0,-.38,p.y);
    sum += fade*(curtain*(.12 + rib*(.20 + .16*signal.z)) + glow)*
      palette(vec3(.28,.21,.70), vec3(.16,.91,.63), .36 + .18*k + .13*rib);
  }
  return sum;
}

float roundedBox(vec2 p, vec2 halfSize, float radius) {
  vec2 q = abs(p) - halfSize + radius;
  return length(max(q, 0.)) + min(max(q.x, q.y), 0.) - radius;
}

float gridLine(float value, float width) {
  float distance = abs(fract(value + .5) - .5);
  return 1. - smoothstep(width, width + clamp(fwidth(value),.001,.18), distance);
}

vec3 moire(vec2 p, float time) {
  vec2 q = rotate(.22 + .19 * sin(time*.37)) * p;
  vec2 center = vec2(.82+.19*signal.x,.42*sin(time*.41));
  q.y += .15*sin(q.x*2.1-time*.72) + .11 * scope(uv.x);
  float a = length(q - center), b = length(q + center);
  float spacing = 18. + 7.*signal.y;
  float waveA = cos(spacing*a - 2.4*drive + 1.3*sin(b*4.-time*.49));
  float waveB = cos(spacing*b + 2.0*drive + 1.2*cos(a*3.6+time*.43));
  float overlap = pow(.5+.5*waveA*waveB,4.);
  float threads = pow(abs((waveA+waveB)*.5),9.);
  float seam = thread(abs(a-b) - .32 - .12*signal.x,.07);
  float crossing = smoothstep(-.65,.85,waveA*waveB) * (.20 + .54 * threads);
  vec3 tint = palette(vec3(.23,.16,.67), vec3(.40,.92,.76), crossing);
  vec3 glints = palette(vec3(.43,.14,.48), vec3(.91,.57,.78), .48);
  return exp(-.12*dot(q,q)) * (tint*(.08 + .25*overlap + .40*threads) +
    glints*seam*(.13 + .19*signal.z));
}

vec3 phosphor(vec2 p, float time) {
  float aspect = size.x / size.y;
  vec2 q = p * (1. + .035 * dot(uv-.5,uv-.5));
  float caseEdge = roundedBox(p, vec2(aspect*.985,.955),.14);
  float screen = roundedBox(q, vec2(aspect*.947,.855),.13);
  float inside = 1. - smoothstep(-.006,.005,screen);
  vec3 phosphorInk = palette(vec3(.08,.36,.27),vec3(.64,1.,.58),.72);
  vec3 caseInk = palette(vec3(.12,.17,.15),vec3(.36,.39,.33),.27);
  vec3 color = caseInk * (thread(caseEdge,.009)*.31 + (1.-smoothstep(-.03,.006,caseEdge))*.055);
  color += phosphorInk * thread(screen,.007) * .21;
  float raster = .78 + .22 * cos(gl_FragCoord.y * PI);
  float graticule = max(gridLine(q.x*5.,.025),gridLine(q.y*5.,.025));
  float marks = max(thread(q.y,.002),thread(q.x,.002)) * (.32 + .68 * gridLine((q.x + q.y)*36.,.10));
  float phase = clamp(q.x / (aspect*1.88) + .5, 0., 1.);
  float base = scope(phase);
  float ghost = .58 * base + .42 * scope(phase + .021);
  float width = .006 + .009 * signal.y;
  float trace = thread(q.y - .23 - base*.98,width);
  float echo = thread(q.y + .27 + ghost*.61,width*.76);
  float persistence = thread(q.y - .23 - base*.98,.047) * (.08 + .22 * signal.x);
  float sweep = thread(q.x - aspect*.93 * sin(time*.64),.12);
  color += inside * raster * phosphorInk * (graticule*.048 + marks*.085 + sweep*.038 +
    trace*(.24 + .45*signal.w) + echo*(.13 + .18*signal.z) + persistence);
  vec2 lamp = p - vec2(aspect*.91,-.905);
  float lights = 0.;
  for (int i=0; i<3; i++) {
    lights += thread(length(lamp + vec2(.06*float(i),0.)), .012) * (.30 + .28 * signal.z);
  }
  return color + phosphorInk*lights;
}

float reel(vec2 p, float angle, float size) {
  float r = length(p), a = atan(p.y,p.x) + angle;
  float spokes = pow(.5+.5*cos(a*6.),14.) * smoothstep(.058,.085,r) * (1.-smoothstep(.125,.145,r));
  float teeth = pow(.5+.5*cos(a*42.),7.) * thread(r - size,.011);
  return .33*thread(r-size,.012) + teeth*.34 + spokes*.62 + .49*thread(r-.055,.007);
}

vec3 cassette(vec2 p, float time) {
  vec2 q = rotate(-.078 + .022*sin(time*.58)) * p;
  q.y += .022 * sin(time*.71) + .022 * dynamics.x;
  float shell = roundedBox(q, vec2(1.08,.55),.075);
  float face = 1. - smoothstep(-.015,.005,shell);
  float paper = roundedBox(q - vec2(0.,.14),vec2(.99,.33),.03);
  float windowEdge = roundedBox(q - vec2(0.,.055),vec2(.75,.194),.115);
  float inWindow = 1. - smoothstep(-.006,.006,windowEdge);
  vec3 chrome = palette(vec3(.19,.22,.27),vec3(.72,.76,.72),.52);
  vec3 label = palette(vec3(.23,.08,.22),vec3(.94,.55,.28),.41);
  vec3 color = chrome * (thread(shell,.009)*.48 + face*.07);
  float sheen = (q.y - .33)*25.;
  color += label * (1.-inWindow) * (1.-smoothstep(-.006,.006,paper)) *
    (.12 + .20 * exp(-sheen*sheen) + .08*thread(q.y + .095,.035));
  color += chrome * thread(windowEdge,.006) * .39;
  vec2 left = q - vec2(-.46,.055), right = q - vec2(.46,.055);
  float winding = .018*sin(drive*.25);
  float reels = reel(left,drive*1.78,.16+winding) + reel(right,drive*2.18,.15-winding);
  float tape = thread(q.y + .11 + .018 * scope(q.x*.67+.5), .013) * (1.-smoothstep(.42,.49,abs(q.x)));
  color += inWindow * chrome * (reels*.58 + tape*(.32 + .29*signal.x));
  float lower = roundedBox(vec2(q.x*(1. + q.y*.6),q.y+.414),vec2(.54,.092),.026);
  color += chrome * (thread(lower,.006)*.28 + face * .05 * smoothstep(.15,.51,abs(q.x)));
  float holes = thread(length(vec2(abs(q.x)-.22,q.y+.405))-.035,.007);
  float screws = thread(length(abs(q)-vec2(.992,.469))-.018,.006);
  float printMarks = thread(q.y-.395,.005) * (1.-smoothstep(.12,.13,abs(q.x+.59)));
  float leds = gridLine((q.x+.28)*15.,.14) * thread(q.y+.42,.007) * step(0.,q.x) * step(q.x,.44*signal.w);
  float grooves = pow(.5+.5*cos(29. * length(p*vec2(.7,1.)) - drive*.43),8.) * exp(-.39*dot(p,p));
  return color + chrome*(.52*screws + .43*holes + .22*printMarks + grooves*.07*(1.-face)) + label*leds*.7;
}

vec3 nacre(vec2 p, float time) {
  vec2 q = rotate(-.37 + .15*sin(time*.40)) * p;
  float drift = .50*sin(q.x*1.63 - time*.62) + .22*cos(q.x*3.6 + time*.78);
  drift += .19*sin(q.x*6.9 - .77*drive + q.y*2.1) * signal.y + .14 * scope(uv.x);
  float surface = q.y + drift + .27*sin(q.y*2.4 - time*.53 + q.x*2.);
  float bands = surface*1.84 + .23*cos(q.x*3.8 + drive*.33);
  float folds = pow(.5+.5*cos(bands*PI*2.),5.);
  float fibers = gridLine(bands*11. + .38*sin(q.x*15. + time*.72),.022);
  float sheen = .5+.5*sin(surface*8. - q.x*2. + time*.61);
  float edge = gridLine(bands,.055) * (.22+.28*signal.z);
  vec3 shell = palette(vec3(.17,.26,.60), vec3(.87,.64,.55),sheen);
  vec3 rim = palette(vec3(.24,.37,.67),vec3(.55,.97,.85),.38+.38*sheen);
  return (shell*(.065+folds*(.24+.16*signal.x)) + rim*(edge + fibers*(.046+.09*signal.z))) *
    (.72 + .28*exp(-.32*dot(q,q)));
}

vec3 vectorTunnel(vec2 p, float time) {
  vec2 q = p - vec2(.19*sin(time*.51)+.13*dynamics.y,.14*cos(time*.41));
  float r = length(q);
  float theta = atan(q.y,q.x) + .19*time + .29*signal.y*sin(r*2.3);
  float angle = mod(theta + PI/6., PI/3.) - PI/6.;
  float polygon = r*cos(angle);
  float depth = log(max(polygon,.018) + .075);
  float sweep = depth*3.1 - drive*.47;
  float gate = gridLine(sweep,.026);
  float rafters = gridLine(theta*6./(2.*PI),.017);
  float etch = gridLine(sweep*3.,.018) * (.12+.17*signal.z);
  float band = smoothstep(.08,.2,polygon) * (.82 + .18*cos(depth*6.2));
  float sparks = pow(.5+.5*cos(theta*24. + drive*2.2),8.) * gate;
  float bloom = thread(fract(sweep+.5)-.5,.11) * (.11+.20*signal.x);
  vec3 frame = palette(vec3(.13,.22,.53),vec3(.85,.50,.23),.46+.34*cos(sweep*2.));
  vec3 beam = palette(vec3(.31,.16,.47),vec3(.42,.82,1.),.57);
  return band * (frame*(gate*.56 + bloom + sparks*.18) + beam*(rafters*.24+etch));
}

vec3 shape(int mode, vec2 p, float time) {
  if (mode == 0) return ink(p, time);
  if (mode == 1) return vortex(p, time);
  if (mode == 2) return silk(p, time);
  if (mode == 3) return interference(p, time);
  if (mode == 4) return orbit(p, time);
  if (mode == 5) return prism(p, time);
  if (mode == 6) return rain(p, time);
  if (mode == 7) return aurora(p, time);
  if (mode == 8) return moire(p,time);
  if (mode == 9) return phosphor(p,time);
  if (mode == 10) return cassette(p,time);
  if (mode == 11) return nacre(p,time);
  return vectorTunnel(p,time);
}

void main() {
  float aspect = size.x/size.y;
  vec2 p = (uv-.5)*2.*vec2(aspect,1.);
  float r = length(p);
  float vintage = mix(previous == 9 || previous == 10 ? 1. : 0., scene == 9 || scene == 10 ? 1. : 0., blend);
  float flow = (.07 + .10*signal.y + .14*sin(clock*.41 + r*3.))*delta*(1.-vintage*.82);
  vec2 warped = rotate(flow) * p * (1. + delta*(.095 + .22*signal.x));
  warped += delta*.025*vec2(sin(p.y*7.+clock), cos(p.x*6.-clock));
  vec3 old = texture(trail, warped/vec2(aspect,1.)*.5+.5).rgb;
  vec3 form = shape(scene,p,clock);
  if (previous != scene) form = mix(shape(previous,p,clock),form,blend);
  float dt = delta * 30.;
  float presence = mix(.32,.45,vintage) + mix(.64,.54,vintage)*signal.w;
  vec3 light = pow(clamp(form * 1.65, 0., .92), vec3(.61)) * presence;
  vec3 feedback = old * exp(-delta*(4.0 + 6.*vintage - 1.15*signal.x));
  vec3 combined = max(light, min(vec3(.82),feedback + light*dt*.14));
  pixel = vec4(combined,1.);
}
