function nabnabCopyDictionary(doc, source) {
    var copy = doc.newDictionary();
    if (source && source.isDictionary()) source.forEach(function (value, key) { copy.put(key, value); });
    return copy;
}
function nabnabAppendOverlay(doc, pageIndex, overlayDoc, overlayIndex) {
    var page = doc.loadPage(pageIndex), overlay = overlayDoc.loadPage(overlayIndex);
    var object = page.getObject(), overlayObject = overlay.getObject();
    var resources = nabnabCopyDictionary(doc, object.getInheritable('Resources'));
    var xobjects = nabnabCopyDictionary(doc, resources.get('XObject'));
    var name = 'NabnabOverlay', suffix = 0;
    while (xobjects.get(name)) name = 'NabnabOverlay' + (++suffix);
    var buffer = new Buffer(), contents = overlayObject.get('Contents');
    function add(stream) { buffer.writeBuffer(stream.readStream()); buffer.write('\n'); }
    if (contents && contents.isArray()) contents.forEach(add); else if (contents) add(contents);
    var graft = doc.newGraftMap();
    var form = doc.addStream(buffer, {
        Type: 'XObject', Subtype: 'Form', FormType: 1,
        BBox: graft.graftObject(overlayObject.getInheritable('MediaBox')),
        Resources: overlayObject.getInheritable('Resources') ? graft.graftObject(overlayObject.getInheritable('Resources')) : {}
    });
    xobjects.put(name, form); resources.put('XObject', xobjects); object.put('Resources', resources);
    var bounds = page.getBounds(), from = overlay.getBounds();
    var transform = Matrix.invert(overlay.getTransform());
    transform = Matrix.concat(transform, Matrix.translate(-from[0], -from[1]));
    transform = Matrix.concat(transform, Matrix.scale((bounds[2]-bounds[0])/(from[2]-from[0]), (bounds[3]-bounds[1])/(from[3]-from[1])));
    transform = Matrix.concat(transform, Matrix.translate(bounds[0], bounds[1]));
    transform = Matrix.concat(transform, page.getTransform());
    var array = doc.newArray(), previous = object.get('Contents');
    array.push(doc.addStream('q\n'));
    if (previous && previous.isArray()) previous.forEach(function (stream) { array.push(stream); });
    else if (previous) array.push(previous);
    array.push(doc.addStream('\nQ\nq\n' + transform.join(' ') + ' cm\n/' + name + ' Do\nQ\n'));
    object.put('Contents', array);
}
