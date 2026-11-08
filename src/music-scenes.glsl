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

vec3 shape(int mode, vec2 p, float time) {
  if (mode == 0) return ink(p, time);
  if (mode == 1) return vortex(p, time);
  if (mode == 2) return silk(p, time);
  if (mode == 3) return interference(p, time);
  if (mode == 4) return orbit(p, time);
  if (mode == 5) return prism(p, time);
  if (mode == 6) return rain(p, time);
  return aurora(p, time);
}

void main() {
  float aspect = size.x/size.y;
  vec2 p = (uv-.5)*2.*vec2(aspect,1.);
  float r = length(p);
  float flow = (.07 + .10*signal.y + .14*sin(clock*.41 + r*3.))*delta;
  vec2 warped = rotate(flow) * p * (1. + delta*(.095 + .22*signal.x));
  warped += delta*.025*vec2(sin(p.y*7.+clock), cos(p.x*6.-clock));
  vec3 old = texture(trail, warped/vec2(aspect,1.)*.5+.5).rgb;
  vec3 form = shape(scene,p,clock);
  if (previous != scene) form = mix(shape(previous,p,clock),form,blend);
  float dt = delta * 30.;
  float presence = .32 + .64*signal.w;
  vec3 light = pow(clamp(form * 1.65, 0., .92), vec3(.61)) * presence;
  vec3 feedback = old * exp(-delta*(4.0 - 1.15*signal.x));
  vec3 combined = max(light, min(vec3(.82),feedback + light*dt*.14));
  pixel = vec4(combined,1.);
}
