export const COLOR_CYCLE_SECONDS = 180;
export const COLOR_WAVE_SECONDS = 13;
export interface ColorWave { x:number; y:number; born:number; strength:number; ellipse:number }
export type Rgb = readonly [number,number,number];
let activations=0;
let fullColor=false;
const listeners=new Set<()=>void>();

export function isFullColor():boolean{return fullColor;}
export function handleEmblemClick():{fullColor:boolean;toggled:boolean}{
  activations=(activations+1)%10;
  const toggled=activations===0;
  if(toggled){fullColor=!fullColor;for(const listener of listeners)listener();}
  return {fullColor,toggled};
}
export function onColorModeChange(listener:()=>void):()=>void{listeners.add(listener);return ()=>{listeners.delete(listener);};}
export function hueRgb(hue:number):Rgb{
  return [hueChannel(hue,0),hueChannel(hue,2/3),hueChannel(hue,1/3)];
}
function hueChannel(hue:number,offset:number):number{return Math.max(0,Math.min(1,Math.abs(((hue+offset)%1+1)%1*6-3)-1));}
export function tintPacked(hue:number,saturation:number,level=255):number{
  if(saturation===0){const gray=Math.round(level);return gray|(gray<<8)|(gray<<16);}
  const red=Math.round(level*(1-saturation*(1-hueChannel(hue,0))));
  const green=Math.round(level*(1-saturation*(1-hueChannel(hue,2/3))));
  const blue=Math.round(level*(1-saturation*(1-hueChannel(hue,1/3))));
  return red|(green<<8)|(blue<<16);
}
export function spectrumAt(time:number,x:number,y:number,width:number,height:number,waves:readonly ColorWave[],full:boolean,reduced:boolean):{hue:number;saturation:number}{
  const base=time/COLOR_CYCLE_SECONDS;
  if(full)return {hue:base+x/width*.55+y/height*.45,saturation:.85};
  let strength=0;
  if(!reduced)for(const wave of waves){
    const age=time-wave.born;
    if(age<0||age>=COLOR_WAVE_SECONDS)continue;
    const distance=Math.hypot(x-wave.x,(y-wave.y)/wave.ellipse);
    const offset=distance-age*47;
    strength=Math.max(strength,Math.exp(-offset*offset/11000)*Math.sin(Math.PI*age/COLOR_WAVE_SECONDS)*wave.strength);
  }
  return {hue:base+x/width*.025+y/height*.02,saturation:Math.min(.62,strength*.72)};
}
export function tintRgb(hue:number,saturation:number):Rgb{
  const rgb=hueRgb(hue);
  return [1-saturation*(1-rgb[0]),1-saturation*(1-rgb[1]),1-saturation*(1-rgb[2])];
}
