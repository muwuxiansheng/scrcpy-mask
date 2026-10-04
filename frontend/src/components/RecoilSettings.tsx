import { useEffect, useRef, useState } from "react";
import { Alert, Button, Input, InputNumber, Select, Space, Switch, Table, Tag } from "antd";
import { ItemBox, ItemBoxContainer } from "./common/ItemBox";
import { requestGet, requestPost } from "../utils";
import { useMessageContext } from "../hooks";

type Stage = { duration_ms: number; x_per_second: number; y_per_second: number };
type Profile = { id: string; name: string; notes: string; calibrated: boolean;
  reference_source: string; reference_ratio: number | null; delay_ms: number; scale: number; stages: Stage[] };
type Config = { enabled: boolean; mode: "same_finger" | "separate_finger"; active_profile_id: string;
  toggle_key: string; next_profile_key: string; sync_weapon_keys: boolean; slot1_profile_id: string;
  slot2_profile_id: string; pointer_id: number; anchor_x: number; anchor_y: number; radius_x: number; radius_y: number;
  profiles: Profile[]; reference_data: unknown };
type Status = { enabled: boolean; mode: string; profile: string; firing: boolean; message: string };
const keys = [...Array.from({length:12},(_,i)=>`F${i+1}`),"Home","End","Insert","Delete"];
const zeroStages = (): Stage[] => [{duration_ms:500,x_per_second:0,y_per_second:0},{duration_ms:4500,x_per_second:0,y_per_second:0}];
const newId = () => `recoil-${Date.now()}-${Math.random().toString(16).slice(2,8)}`;

export default function RecoilSettings() {
  const [config,setConfig] = useState<Config|null>(null);
  const [status,setStatus] = useState<Status|null>(null);
  const [saving,setSaving] = useState(false);
  const dirty = useRef(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const messages = useMessageContext();
  useEffect(()=> {
    let disposed=false;
    async function load() {
      try {
        const [c,s]=await Promise.all([requestGet<{recoil:Config}>("/api/config/get_config"),requestGet<Status>("/api/config/recoil_status")]);
        if (!disposed) { if(!dirty.current)setConfig(c.data.recoil);setStatus(s.data); }
      } catch { /* The surrounding settings page reports service connectivity. */ }
    }
    void load();const timer=setInterval(load,2000);
    return()=>{disposed=true;clearInterval(timer);};
  },[]);
  if(!config)return <section><h3 className="title-with-line-sub">压枪控制</h3><p>正在读取压枪设置…</p></section>;
  const cfg=config;
  const profile=cfg.profiles.find(p=>p.id===cfg.active_profile_id)??cfg.profiles[0];
  const options=cfg.profiles.map(p=>({value:p.id,label:`${p.name}${p.calibrated?"":" · 未校准"}`}));
  function edit(patch:Partial<Config>) {dirty.current=true;setConfig({...cfg,...patch});}
  function editProfile(patch:Partial<Profile>) { edit({profiles:cfg.profiles.map(p=>p.id===profile.id?{...p,...patch}:p)}); }
  function editStage(index:number,patch:Partial<Stage>) {editProfile({stages:profile.stages.map((s,i)=>i===index?{...s,...patch}:s)});}
  async function save() {
    setSaving(true);
    try {const res=await requestPost("/api/config/update_config",{key:"recoil",value:cfg});dirty.current=false;messages?.success(res.message);}
    catch(e){messages?.error(String(e));}finally{setSaving(false);}
  }
  function exportData() {
    const url=URL.createObjectURL(new Blob([JSON.stringify({version:1,recoil:cfg},null,2)],{type:"application/json"}));
    const a=document.createElement("a");a.href=url;a.download="压枪方案.json";a.click();URL.revokeObjectURL(url);
  }
  async function importData(file:File) {
    try {
      if(file.size>1024*1024)throw new Error("配置文件不能超过1MB");
      const data=JSON.parse(await file.text());
      if(Array.isArray(data.moveList)) {
        const profiles:Profile[]=data.moveList.filter((g:any)=>typeof g.baseName==="string"&&g.baseName.trim()).map((g:any)=>({
          id:newId(),name:g.baseName,notes:"导入极限压枪参考：原系数单位未确认，需实测填写手机像素/秒。记录倍镜、配件、姿态和灵敏度。",
          calibrated:false,reference_source:`极限压枪参考：${file.name}`,reference_ratio:Number(g.baseRatio),
          delay_ms:0,scale:1,stages:zeroStages(),
        }));
        if(cfg.profiles.length+profiles.length>128)throw new Error("导入后超过128个方案，请先精简原配置");
        edit({profiles:[...cfg.profiles,...profiles],reference_data:data});
        messages?.success(`已读取${profiles.length}个武器模板；补偿速度为0，保存后可逐个校准`);
      } else {
        const imported=data.recoil??data;
        if(!Array.isArray(imported.profiles)||!imported.profiles.length)throw new Error("不是压枪方案或极限压枪配置");
        const profiles:Profile[]=imported.profiles.map((p:any)=>{
          if(typeof p.name!=="string"||!Array.isArray(p.stages)||!p.stages.length
            ||p.stages.some((s:any)=>![s.duration_ms,s.x_per_second,s.y_per_second].every(Number.isFinite))) {
            throw new Error("方案名称或阶段数据不完整");
          }
          return {notes:"",calibrated:false,reference_source:"",reference_ratio:null,delay_ms:0,scale:1,...p,id:p.id||newId()};
        });
        const next:Config={...cfg,...imported,profiles,enabled:false};
        if(!profiles.some(p=>p.id===next.active_profile_id))next.active_profile_id=profiles[0].id;
        if(!profiles.some(p=>p.id===next.slot1_profile_id))next.slot1_profile_id="";
        if(!profiles.some(p=>p.id===next.slot2_profile_id))next.slot2_profile_id="";
        dirty.current=true;setConfig(next);messages?.success("已读取方案；保存后生效，开关暂时关闭");
      }
    }catch(e){messages?.error(String(e));}
  }
  return <section>
    <h3 className="title-with-line-sub">压枪控制</h3>
    <Space wrap className="mb-3">
      <Tag color={status?.enabled?"blue":"default"}>{status?.enabled?"已开启":"已关闭"}</Tag>
      <span>当前：{status?.profile} · {status?.mode} · {status?.message}</span>
      {status?.firing&&<Tag color="green">补偿中</Tag>}
      {dirty.current&&<Tag color="orange">有未保存修改</Tag>}
    </Space>
    <Alert type="info" showIcon className="mb-3" message="武器数据须实测校准"
      description="极限压枪的系数只能作为参考，不能直接当作手机触摸像素。先在训练场按同一套倍镜、配件、姿态和灵敏度校准；同指和双指可能需要不同数值。向下为正，向上为负。"/>
    <ItemBoxContainer className="mb-4">
      <ItemBox label="压枪开关" tooltip="保存后生效。只有实际开火且处于FPS视角时才运行；松开、失焦、自由视角、手机指针、F10复位均停止。">
        <Switch checked={cfg.enabled} onChange={v=>edit({enabled:v})}/>
      </ItemBox>
      <ItemBox label="触摸方式">
        <Select className="w-full" value={cfg.mode} onChange={mode=>edit({mode})} options={[
          {value:"same_finger",label:"同指：叠加到现有视角触点"},{value:"separate_finger",label:"双指：独立触点下滑，保留原视角触点"}]}/>
      </ItemBox>
      <ItemBox label="开关快捷键"><Select className="w-full" value={cfg.toggle_key} options={keys.map(value=>({value,label:value}))} onChange={toggle_key=>edit({toggle_key})}/></ItemBox>
      <ItemBox label="下一个方案快捷键"><Select className="w-full" value={cfg.next_profile_key} options={keys.map(value=>({value,label:value}))} onChange={next_profile_key=>edit({next_profile_key})}/></ItemBox>
      <ItemBox label="当前武器方案"><Select showSearch optionFilterProp="label" className="w-full" value={cfg.active_profile_id} options={options} onChange={active_profile_id=>edit({active_profile_id})}/></ItemBox>
      <ItemBox label="数字1/2同步武器方案"><Switch checked={cfg.sync_weapon_keys} onChange={sync_weapon_keys=>edit({sync_weapon_keys})}/></ItemBox>
      <ItemBox label="数字1对应方案"><Select showSearch optionFilterProp="label" className="w-full" value={cfg.slot1_profile_id} options={[{value:"",label:"不切换"},...options]} onChange={slot1_profile_id=>edit({slot1_profile_id})}/></ItemBox>
      <ItemBox label="数字2对应方案"><Select showSearch optionFilterProp="label" className="w-full" value={cfg.slot2_profile_id} options={[{value:"",label:"不切换"},...options]} onChange={slot2_profile_id=>edit({slot2_profile_id})}/></ItemBox>
      {cfg.mode==="separate_finger"&&<>
        <ItemBox label="独立触点ID" tooltip="须避开其他映射和脚本的触点ID；默认90，检测到冲突时不会注入。"><InputNumber className="w-full" min={0} max={65535} precision={0} value={cfg.pointer_id} onChange={v=>v!==null&&edit({pointer_id:v})}/></ItemBox>
        <ItemBox label="独立起点 X（屏幕百分比）"><InputNumber className="w-full" min={1} max={99} value={cfg.anchor_x*100} onChange={v=>v!==null&&edit({anchor_x:v/100})}/></ItemBox>
        <ItemBox label="独立起点 Y（屏幕百分比）"><InputNumber className="w-full" min={1} max={99} value={cfg.anchor_y*100} onChange={v=>v!==null&&edit({anchor_y:v/100})}/></ItemBox>
        <ItemBox label="独立滑动 X 范围（手机像素）"><InputNumber className="w-full" min={1} max={1000} value={cfg.radius_x} onChange={v=>v!==null&&edit({radius_x:v})}/></ItemBox>
        <ItemBox label="独立滑动 Y 范围（手机像素）" tooltip="触点到边界后抬起并回到起点继续滑动；不同游戏可能忽略第二根手指。"><InputNumber className="w-full" min={1} max={1000} value={cfg.radius_y} onChange={v=>v!==null&&edit({radius_y:v})}/></ItemBox>
      </>}
    </ItemBoxContainer>
    <h4>编辑方案</h4>
    <ItemBoxContainer>
      <ItemBox label="方案名称"><Input value={profile.name} onChange={e=>editProfile({name:e.target.value})}/></ItemBox>
      <ItemBox label="条件与校准记录"><Input.TextArea rows={3} value={profile.notes} placeholder="武器、倍镜、配件、姿态、灵敏度、同指或双指" onChange={e=>editProfile({notes:e.target.value})}/></ItemBox>
      <ItemBox label="已在本设备校准"><Switch checked={profile.calibrated} onChange={calibrated=>editProfile({calibrated})}/></ItemBox>
      <ItemBox label="开始补偿延迟（毫秒）"><InputNumber className="w-full" min={0} max={10000} precision={0} value={profile.delay_ms} onChange={v=>v!==null&&editProfile({delay_ms:v})}/></ItemBox>
      <ItemBox label="总力度倍率" tooltip="倍率只乘本方案的手机像素速度，不自动套用极限压枪的未知单位系数。"><InputNumber className="w-full" min={0} max={20} step={0.05} value={profile.scale} onChange={v=>v!==null&&editProfile({scale:v})}/></ItemBox>
    </ItemBoxContainer>
    {profile.reference_source&&<p>参考来源：{profile.reference_source}；原始基础系数：{profile.reference_ratio}（仅记录，不直接执行）</p>}
    <p>阶段按顺序执行，完成后停止补偿，直到松开再开火。速度单位：手机原始坐标像素／秒。</p>
    <Table<Stage> size="small" pagination={false} rowKey={(_,i)=>String(i)} dataSource={profile.stages} columns={[
      {title:"阶段",render:(_,__,i)=>i+1},
      {title:"持续毫秒",render:(_,s,i)=><InputNumber min={1} max={60000} precision={0} value={s.duration_ms} onChange={v=>v!==null&&editStage(i,{duration_ms:v})}/>},
      {title:"X速度（右为正）",render:(_,s,i)=><InputNumber min={-10000} max={10000} step={1} value={s.x_per_second} onChange={v=>v!==null&&editStage(i,{x_per_second:v})}/>},
      {title:"Y速度（下为正）",render:(_,s,i)=><InputNumber min={-10000} max={10000} step={1} value={s.y_per_second} onChange={v=>v!==null&&editStage(i,{y_per_second:v})}/>},
      {title:"操作",render:(_,__,i)=><Button size="small" danger disabled={profile.stages.length===1} onClick={()=>editProfile({stages:profile.stages.filter((_,j)=>i!==j)})}>删除</Button>},
    ]}/>
    <Space wrap className="my-3">
      <Button disabled={profile.stages.length>=64} onClick={()=>editProfile({stages:[...profile.stages,{duration_ms:1000,x_per_second:0,y_per_second:0}]})}>添加阶段</Button>
      <Button onClick={()=>{const p={...profile,id:newId(),name:`${profile.name} 副本`,stages:profile.stages.map(s=>({...s}))};edit({profiles:[...cfg.profiles,p],active_profile_id:p.id});}}>复制方案</Button>
      <Button danger disabled={cfg.profiles.length===1} onClick={()=>{const ps=cfg.profiles.filter(p=>p.id!==profile.id);edit({profiles:ps,active_profile_id:ps[0].id,slot1_profile_id:cfg.slot1_profile_id===profile.id?"":cfg.slot1_profile_id,slot2_profile_id:cfg.slot2_profile_id===profile.id?"":cfg.slot2_profile_id});}}>删除方案</Button>
      <Button onClick={()=>fileInput.current?.click()}>导入方案／极限压枪JSON</Button>
      <Button onClick={exportData}>导出方案</Button>
      <Button type="primary" loading={saving} onClick={save}>保存压枪设置</Button>
    </Space>
    <input ref={fileInput} type="file" accept=".json,application/json" hidden onChange={e=>{const file=e.target.files?.[0];if(file)void importData(file);e.target.value="";}}/>
  </section>;
}
