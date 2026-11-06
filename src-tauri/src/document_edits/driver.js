var mode=scriptArgs[0], input=scriptArgs[1];
var doc=Document.openDocument(input);
if (!doc.isPDF() || doc.needsPassword()) throw new Error('Choose an authorized PDF that does not require a password.');
doc.disableJS();
var count=doc.countPages();
if (count<1 || count>10000) throw new Error('Choose a PDF of 1-10000 pages.');
function information() {
    var root=doc.getTrailer().get('Root'), acro=root.get('AcroForm');
    var signatures=!!root.get('Perms'), forms=!!acro, inspected=0, visited={};
    function field(node,depth) {
        if (!node || depth>16 || ++inspected>2000) return;
        if (node.isIndirect()) {var key=node.asIndirect();if(visited[key])return;visited[key]=true;}
        var ft=node.get('FT');if(ft && ft.asName()==='Sig')signatures=true;
        var kids=node.get('Kids');if(kids && kids.isArray())kids.forEach(function(k){field(k,depth+1);});
    }
    var fields=acro?acro.get('Fields'):null;if(fields && fields.isArray())fields.forEach(function(f){field(f,0);});
    var bookmarks=[], truncated=false;
    function outline(nodes,level) {
        if (!nodes) return;
        for(var i=0;i<nodes.length;i++) {
            if(bookmarks.length>=500 || level>4){truncated=true;return;}
            var node=nodes[i], page=node.page;
            if(typeof page!=='number' && node.uri)page=doc.resolveLink(node.uri);
            if(typeof page==='number' && page>=0 && page<count)bookmarks.push({title:String(node.title||'Bookmark').replace(/[\x00-\x1f\x7f]/g,' ').slice(0,200),page:page+1,level:level});
            else truncated=true;
            if(node.down)outline(node.down,level+1);
        }
    }
    outline(doc.loadOutline(),1);
    var bounds=doc.loadPage(0).getBounds();
    return {pages:count,width:bounds[2]-bounds[0],height:bounds[3]-bounds[1],forms:forms,signatures:signatures,edit:doc.hasPermission('edit'),copy:doc.hasPermission('copy'),print:doc.hasPermission('print'),bookmarks:bookmarks,bookmarksTruncated:truncated||inspected>2000};
}
if(mode==='inspect')print(JSON.stringify(information()));
else {
    var request=JSON.parse(readFile(scriptArgs[2]).asString()), options=request.options, selected=request.selectedPages;
    var output=scriptArgs[3], overlayPath=scriptArgs[4];
    if(!doc.hasPermission('edit'))throw new Error('This PDF does not permit editing.');
    if(options.kind==='crop') {
        for(var n=0;n<selected.length;n++) {
            var page=doc.loadPage(selected[n]-1),bounds=page.getBounds();
            var crop=[bounds[0]+options.left,bounds[1]+options.top,bounds[2]-options.right,bounds[3]-options.bottom];
            if(crop[2]-crop[0]<12 || crop[3]-crop[1]<12)throw new Error('Margins leave less than 12 points of visible page.');
            page.setPageBox('CropBox',crop);
        }
    } else if(options.kind==='bookmarks') {
        var root=doc.getTrailer().get('Root');
        if(!options.entries.length)root.delete('Outlines');
        else {
            var outline=doc.addObject({Type:'Outlines'}), frames=[{object:outline,last:null,count:0}];
            for(var n=0;n<options.entries.length;n++) {
                var entry=options.entries[n];frames.length=entry.level;
                var parent=frames[entry.level-1];
                var object=doc.addObject({Title:doc.newString(entry.title),Parent:parent.object,Dest:[doc.findPage(entry.page-1),'Fit']});
                if(parent.last){parent.last.put('Next',object);object.put('Prev',parent.last);}else parent.object.put('First',object);
                parent.object.put('Last',object);parent.last=object;
                for(var depth=0;depth<frames.length;depth++){frames[depth].count++;frames[depth].object.put('Count',frames[depth].count);}
                frames.push({object:object,last:null,count:0});
            }
            root.put('Outlines',outline);
        }
    } else if(options.kind==='number' || options.kind==='watermark') {
        var writer=new DocumentWriter(overlayPath,'pdf','compress'), font=new Font('Helvetica');
        for(var n=0;n<selected.length;n++) {
            var page=doc.loadPage(selected[n]-1),bounds=page.getBounds(), width=bounds[2]-bounds[0],height=bounds[3]-bounds[1];
            var device=writer.beginPage(bounds), text=new Text();
            var label=options.kind==='number'?options.prefix+String(options.start+n):options.text;
            text.showString(font,[options.fontSize,0,0,-options.fontSize,0,0],label);
            var rect=text.getBounds(null,Matrix.identity), textWidth=rect[2]-rect[0];
            if(textWidth>width-24 || rect[3]-rect[1]>height-24)throw new Error('The label does not fit this page. Reduce font size or text.');
            var x=bounds[0]+(width-textWidth)/2-rect[0];
            var y=options.kind==='watermark'?bounds[1]+height/2-(rect[1]+rect[3])/2:options.position==='top'?bounds[1]+12-rect[1]:bounds[3]-12-rect[3];
            device.fillText(text,Matrix.translate(x,y),ColorSpace.DeviceGray,[0],options.kind==='watermark'?options.opacity/100:1);
            writer.endPage();
        }
        writer.close();var overlays=Document.openDocument(overlayPath);
        for(var n=0;n<selected.length;n++)nabnabAppendOverlay(doc,selected[n]-1,overlays,n);
    } else throw new Error('Unsupported PDF edit operation.');
    doc.save(output,'garbage=deduplicate,compress,compress-fonts,compress-images');
    print(JSON.stringify({pages:count}));
}
