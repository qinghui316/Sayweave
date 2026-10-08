/** Local, interruptible springs for selection surfaces; no business state lives here. */
export interface SpringBody {position:number[];velocity:number[];target:number[];delay?:number}
export const body=(values:number[]):SpringBody=>({position:[...values],velocity:values.map(()=>0),target:[...values]});
export function advance(body:SpringBody,seconds:number,stiffness=580,damping=.75){
  // Bounded integration remains stable after dropped browser frames.
  const steps=Math.max(1,Math.ceil(seconds*120)),dt=seconds/steps;
  for(let step=0;step<steps;step++)for(let i=0;i<body.position.length;i++){
    const position=body.position[i]!,velocity=body.velocity[i]!,target=body.target[i]!;
    body.velocity[i]=velocity+((target-position)*stiffness-2*damping*Math.sqrt(stiffness)*velocity)*dt;
    body.position[i]=position+body.velocity[i]!*dt;
  }
}
export function settle(body:SpringBody){body.position=[...body.target];body.velocity.fill(0);}
export function runSelectionSpring(items:{body:SpringBody;paint:()=>void;stiffness?:number}[]){
  let frame=0,last=performance.now();const started=last;
  const tick=(now:number)=>{
    const dt=Math.min(.032,Math.max(0,(now-last)/1000));last=now;let busy=false;
    for(const item of items){
      if(now-started<(item.body.delay??0)){busy=true;continue;}
      advance(item.body,dt,item.stiffness);
      const moving=item.body.position.some((v,i)=>Math.abs(v-item.body.target[i]!)>.0005||Math.abs(item.body.velocity[i]!)>.005);
      if(!moving)settle(item.body);else busy=true;
      item.paint();
    }
    if(busy&&now-started<1200)frame=requestAnimationFrame(tick);
    else for(const item of items){settle(item.body);item.paint();}
  };
  frame=requestAnimationFrame(tick);
  return()=>cancelAnimationFrame(frame);
}
