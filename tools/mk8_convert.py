"""Keep-proportions variant of mixamo_to_skate/converter.py for tools/mk8_character.py. Kept separate: the character customiser fingerprint covers mixamo_to_skate/*.py, so edits there send players back through setup."""
import copy
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

import numpy as np

from glb import ConversionError, Document, Writer, require, clean_node_transform

VERSION = 1
MAP = {'hips':'HIPS', 'spine':'SPINE', 'spine1':'SPINE1', 'spine2':'SPINE3',
       'neck':'NECK', 'head':'HEAD'}
for side in ('left', 'right'):
    for part in ('shoulder', 'arm', 'forearm', 'hand', 'upleg', 'leg', 'foot', 'toebase'):
        MAP[side+part] = (side+part).upper()
REQUIRED = set(MAP) - {'lefttoebase', 'righttoebase'}
BOARD = {'SKATEBOARD_ROOT', 'TRUCK_FRONT', 'TRUCK_BACK', 'LEFT_WHEELFRONT',
         'RIGHT_WHEELFRONT', 'LEFT_WHEELBACK', 'RIGHT_WHEELBACK'}


# Secondary joints kept (as JIGGLE_<name>) for the engine's spring bones:
# hair, tails, skirts, ears, caps, mustaches, sleeves, tongues, ribbons, bags.
JIGGLE = ('hair', 'tail', 'skirt', 'ear', 'cap', 'mustache', 'sleeve', 'seleeve', 'tongue', 'ribbon', 'ponytail', 'bag')


# Jiggle joints that spin freely about their bone (Wendy's arm rings), set by
# the caller per character: JIGGLE_SPIN_<name>.
SPIN = set()


def jiggle_name(name):
    base = name.rsplit(':', 1)[-1]
    if not any(k in base.lower() for k in JIGGLE) or base.lower().startswith('mixamorig'):
        return None
    return ('JIGGLE_SPIN_' if any(k in base.lower() for k in SPIN) else 'JIGGLE_') + base


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def key(name):
    # Mixamo prefixes may include a namespace number. Do not guess arbitrary rigs.
    name = name.rsplit(':', 1)[-1].lower()
    return name


def named_nodes(doc):
    result = {}
    for i, n in enumerate(doc.doc['nodes']):
        k = key(n.get('name', ''))
        if k in MAP or k.startswith(('lefthand', 'righthand')) or k.endswith('_end'):
            require(k not in result, 'Ambiguous duplicated bone: '+k)
            result[k] = i
    require(REQUIRED <= result.keys(), 'Not a supported Mixamo humanoid; missing: '+
            ', '.join(sorted(REQUIRED-result.keys())))
    for k in REQUIRED - {'hips'}:
        parent = doc.parents.get(result[k])
        ancestors = set()
        while parent is not None:
            ancestors.add(parent)
            parent = doc.parents.get(parent)
        require(result['hips'] in ancestors, k+' is outside the hip hierarchy')
    return result


def bind_worlds(doc):
    """Derive bind globals from skin matrices, not frame-zero animation."""
    result = {}
    for ni, node in enumerate(doc.doc['nodes']):
        if 'mesh' not in node:
            continue
        require('skin' in node, 'Unskinned mesh: '+node.get('name', str(ni)))
        skin = doc.doc['skins'][node['skin']]
        require('inverseBindMatrices' in skin, 'Missing inverse bind matrices')
        matrices = doc.accessor(skin['inverseBindMatrices'])
        require(len(matrices) == len(skin['joints']), 'Inverse bind count mismatch')
        for ji, joint in enumerate(skin['joints']):
            inverse = matrices[ji].reshape(4, 4).T
            require(abs(np.linalg.det(inverse)) > 1e-12, 'Singular skin bind')
            world = doc.worlds[ni] @ np.linalg.inv(inverse)
            if joint in result:
                require(np.allclose(result[joint], world, atol=1e-5, rtol=1e-4),
                        'Meshes disagree about a bone bind pose')
            result[joint] = world
    # Unweighted endpoints inherit the nearest known bind transform.
    def solve(i):
        if i in result:
            return result[i]
        parent = doc.parents.get(i)
        result[i] = (solve(parent) @ np.linalg.inv(doc.worlds[parent]) @ doc.worlds[i]
                     if parent is not None else doc.worlds[i])
        return result[i]
    for i in range(len(doc.doc['nodes'])):
        solve(i)
    return result


def mesh_bounds(doc, exclude_board=False):
    points = []
    for ni, node in enumerate(doc.doc['nodes']):
        if 'mesh' not in node:
            continue
        for p in doc.doc['meshes'][node['mesh']]['primitives']:
            name = doc.doc.get('materials', [])[p['material']].get('name', '') if 'material' in p else ''
            if exclude_board and name in ('Retail_SkateBoard', 'Retail_SkateTruck', 'Retail_SkateWheel'):
                continue
            pos = doc.accessor(p['attributes']['POSITION'])
            points.append(pos @ doc.worlds[ni][:3, :3].T + doc.worlds[ni][:3, 3])
    require(points, 'No character geometry')
    return np.concatenate(points)


def normalization(doc, ref, binds, names):
    """Resolve body axes from landmarks, then height/ground, avoiding unit guesses."""
    p = lambda k: binds[names[k]][:3, 3]
    up = p('head')-p('hips')
    require(np.linalg.norm(up)>1e-8, 'Degenerate head/hip landmarks')
    up /= np.linalg.norm(up)
    right = p('leftarm')-p('rightarm')  # stock anatomical left is positive X
    right -= up*np.dot(right, up)
    require(np.linalg.norm(right) > 1e-8, 'Degenerate shoulder landmarks')
    right /= np.linalg.norm(right)
    forward = np.cross(right, up)
    rotation = np.stack([right, up, forward])
    points = mesh_bounds(doc) @ rotation.T
    target = mesh_bounds(ref, True)
    height = np.ptp(points[:, 1])
    require(height > 1e-6, 'Degenerate character height')
    scale = np.ptp(target[:, 1])/height
    matrix = np.eye(4)
    matrix[:3, :3] = rotation*scale
    hip = rotation @ p('hips')*scale
    matrix[:3, 3] = [-hip[0], target[:, 1].min()-points[:, 1].min()*scale, -hip[2]]
    return matrix, scale


def source_skin(doc, primitive, skin):
    attrs = primitive['attributes']
    require('JOINTS_0' in attrs and 'WEIGHTS_0' in attrs, 'Mesh has no skin weights')
    require(not any(k.startswith(('JOINTS_', 'WEIGHTS_')) and k[-1] not in '01' for k in attrs),
            'More than eight source influences are unsupported')
    js, ws = [], []
    for channel in (0, 1):
        j, w = 'JOINTS_'+str(channel), 'WEIGHTS_'+str(channel)
        require((j in attrs) == (w in attrs), 'Unpaired skin attributes')
        if j in attrs:
            js.append(doc.accessor(attrs[j]).astype(np.int64))
            ws.append(doc.accessor(attrs[w]).astype(float))
    joints, weights = np.concatenate(js, axis=1), np.concatenate(ws, axis=1)
    require((joints >= 0).all() and (joints < len(skin['joints'])).all(), 'Invalid joint index')
    require((weights >= 0).all() and (weights.sum(axis=1) > 1e-8).all(), 'Invalid skin weights')
    return joints, weights/weights.sum(axis=1, keepdims=True)


def reference_rig(ref):
    require(len(ref.doc.get('skins', [])) == 1, 'Stock reference must have one shared skin')
    skin = ref.doc['skins'][0]
    names = [ref.doc['nodes'][i].get('name', '').upper() for i in skin['joints']]
    require(len(names) == len(set(names)) and set(MAP.values()) <= set(names),
            'Reference is not a complete supported stock render rig')
    require(len(names) <= 256, 'Stock rig exceeds renderer joint limit')
    inverse = ref.accessor(skin['inverseBindMatrices']).reshape(-1, 4, 4).transpose(0, 2, 1)
    require(len(inverse) == len(names), 'Stock inverse bind count mismatch')
    for j, inv in zip(skin['joints'], inverse):
        require(np.allclose(ref.worlds[j] @ inv, np.eye(4), atol=2e-4),
                'Stock reference is not in its mesh bind pose')
    return skin, names, inverse


def create_profile(source, reference):
    doc, ref = Document(source), Document(reference)
    reference_rig(ref)
    names, binds = named_nodes(doc), bind_worlds(doc)
    norm, scale = normalization(doc, ref, binds, names)
    # Round-trip pairing is meaningful only when the same exported mesh returned.
    # Bounding-box calibration avoids hip fitting shifts in the paired source.
    src_points, ref_points = mesh_bounds(doc), mesh_bounds(ref, True)
    src_box = np.stack([src_points.min(axis=0), src_points.max(axis=0)])
    ref_box = np.stack([ref_points.min(axis=0), ref_points.max(axis=0)])
    scale = float(np.ptp(ref_points[:,1])/np.ptp(src_points[:,1]))
    fit = np.eye(4)
    fit[:3, :3] *= scale
    fit[:3, 3] = ref_box.mean(axis=0)-src_box.mean(axis=0)*scale
    require(np.max(np.abs((src_box*scale+fit[:3, 3])-ref_box)) < 0.015,
            'Calibration is not the returned stock mesh (bounds differ by over 1.5 cm)')
    return {'version':VERSION, 'reference_sha256':sha(reference), 'source_sha256':sha(source),
            'normalization_scale':scale,
            'bones':{k:(fit@binds[i]).tolist() for k, i in names.items()},
            'note':'Private paired-stock calibration; contains no mesh or textures.'}


def fallback_targets(ref, names, binds, norm):
    """Anatomical aim frames for non-calibrated use. Never apply stock bone roll to Mixamo axes."""
    _, target_names, inverse = reference_rig(ref)
    stock = {n:np.linalg.inv(m) for n,m in zip(target_names, inverse)}
    child = {'hips':'spine', 'spine':'spine1', 'spine1':'spine2', 'spine2':'neck',
             'neck':'head'}
    for s in ('left', 'right'):
        child.update({s+'shoulder':s+'arm', s+'arm':s+'forearm', s+'forearm':s+'hand',
                      s+'upleg':s+'leg', s+'leg':s+'foot', s+'foot':s+'toebase'})
    targets = {}
    def rotation_between(a, b):
        a, b = a/np.linalg.norm(a), b/np.linalg.norm(b)
        v, c = np.cross(a, b), np.dot(a, b)
        require(c > -0.99, 'Reversed limb in reference pose; export Mixamo T-pose')
        k = np.array([[0,-v[2],v[1]], [v[2],0,-v[0]], [-v[1],v[0],0]])
        return np.eye(3)+k+k@k/(1+c)
    for k, target in MAP.items():
        if k not in names:
            continue
        src = norm@binds[names[k]]
        src[:3,:3] /= np.linalg.norm(src[:3,:3],axis=0)
        result = src.copy()
        result[:3, 3] = stock[target][:3, 3]
        if k in child and child[k] in names:
            src_direction = (norm@binds[names[child[k]]])[:3,3]-src[:3,3]
            dst_direction = stock[MAP[child[k]]][:3,3]-result[:3,3]
            if min(np.linalg.norm(src_direction), np.linalg.norm(dst_direction)) > 1e-6:
                result[:3,:3] = rotation_between(src_direction, dst_direction)@src[:3,:3]
        targets[k] = result
    return targets


CHAIN = {'hips':'spine', 'spine':'spine1', 'spine1':'spine2', 'spine2':'neck', 'neck':'head'}
for _side in ('left', 'right'):
    CHAIN.update({_side+'shoulder':_side+'arm', _side+'arm':_side+'forearm', _side+'forearm':_side+'hand',
                  _side+'upleg':_side+'leg', _side+'leg':_side+'foot', _side+'foot':_side+'toebase'})
PARENT = {child: parent for parent, child in CHAIN.items()}
for _side in ('left', 'right'):
    PARENT[_side+'shoulder'] = 'spine2'
    PARENT[_side+'upleg'] = 'hips'


def keep_proportion_targets(ref, target_names, names, binds, norm):
    """Re-aim the source's limbs along the stock rest directions while keeping
    its bone lengths. Returns the mesh fit per source bone and the new world
    frame per stock joint (stock orientation, so stock animation rotations
    apply unchanged). Joints the source lacks are interpolated; board joints
    stay stock."""
    joints = ref.doc['skins'][0]['joints']
    stock = {n: ref.worlds[j].copy() for n, j in zip(target_names, joints)}
    for m in stock.values():
        m[:3, :3] /= np.linalg.norm(m[:3, :3], axis=0)
    src = {}
    for k, i in names.items():
        m = norm@binds[i]
        m[:3, :3] /= np.linalg.norm(m[:3, :3], axis=0)
        src[k] = m

    def aim(a, b):
        a, b = a/np.linalg.norm(a), b/np.linalg.norm(b)
        v, c = np.cross(a, b), np.dot(a, b)
        if c < -0.999:
            return np.diag([1., -1., -1.])
        k = np.array([[0, -v[2], v[1]], [v[2], 0, -v[0]], [-v[1], v[0], 0]])
        return np.eye(3)+k+k@k/(1+c)
    delta, position = {}, {}
    order = ['hips'] + [k for k in ('spine', 'spine1', 'spine2', 'neck', 'head') if k in src]
    for side in ('left', 'right'):
        order += [side+p for p in ('shoulder', 'arm', 'forearm', 'hand', 'upleg', 'leg', 'foot', 'toebase')]
    for k in order:
        if k not in src:
            continue
        parent = PARENT.get(k)
        if parent in delta:
            position[k] = position[parent] + delta[parent]@(src[k][:3, 3]-src[parent][:3, 3])
        else:
            position[k] = src[k][:3, 3]
        child = CHAIN.get(k)
        rotation = np.eye(3)
        if child in src and child in MAP and k in MAP:
            rotation = aim(src[child][:3, 3]-src[k][:3, 3], stock[MAP[child]][:3, 3]-stock[MAP[k]][:3, 3])
        elif parent in delta:
            rotation = delta[parent]
        delta[k] = rotation
    fits = {}
    for k in position:
        m = np.eye(4)
        m[:3, :3] = delta[k]@src[k][:3, :3]
        m[:3, 3] = position[k]
        fits[k] = m
    worlds = {}
    placed = {MAP[k]: position[k] for k in position if k in MAP}
    for n in target_names:
        m = stock[n].copy()
        if n in placed:
            m[:3, 3] = placed[n]
        elif n == 'SPINE2':
            m[:3, 3] = (placed['SPINE1']+placed['SPINE3'])/2
        elif n == 'NECK1':
            m[:3, 3] = (placed['NECK']+placed['HEAD'])/2
        elif n.endswith('TOEBASE') and not n.endswith('_REPARENTED'):
            foot = n.replace('TOEBASE', 'FOOT')
            m[:3, 3] = placed[foot]+(stock[n][:3, 3]-stock[foot][:3, 3])*0.6
        worlds[n] = m
    return fits, worlds


def convert_glb(source, reference, output, profile=None, include_board=True, keep_height=None):
    doc, ref = Document(source), Document(reference)
    ref_skin, target_names, inverse = reference_rig(ref)
    # The stock export contains ~1e-16 numerical residue in the affine row.
    # glTF requires this row to be exactly (0,0,0,1).
    require(np.allclose(inverse[:,3,:],[0,0,0,1],atol=1e-7), 'Non-affine stock inverse binds')
    inverse[:,3,:] = [0,0,0,1]
    names, binds = named_nodes(doc), bind_worlds(doc)
    norm, scale = normalization(doc, ref, binds, names)
    report = {'version':VERSION, 'source_sha256':sha(source), 'reference_sha256':sha(reference),
              'mode':'fit_to_stock_proportions', 'scale':float(scale), 'warnings':[],
              'vertices':0, 'triangles':0, 'collapsed_bones':{}, 'max_discarded_weight':0.0}
    if profile:
        require(profile.get('version') == VERSION and profile['reference_sha256'] == sha(reference),
                'Calibration belongs to another stock reference or converter version')
        targets = {k:np.asarray(v,dtype=float) for k,v in profile['bones'].items()}
        require(REQUIRED <= targets.keys(), 'Incomplete calibration')
        # Calibration is world-space in metres; strip source export scale from its bone axes.
        for m in targets.values():
            require(m.shape == (4,4) and np.isfinite(m).all(), 'Invalid calibration matrix')
            m[:3,:3] /= np.linalg.norm(m[:3,:3], axis=0)
        target_length = np.linalg.norm(targets['head'][:3,3]-targets['hips'][:3,3])
        source_length = np.linalg.norm(binds[names['head']][:3,3]-binds[names['hips']][:3,3])
        fitted_scale = target_length/source_length
        norm[:3,:] *= fitted_scale/scale
        scale = fitted_scale
        report['scale'] = float(scale)
        report['calibrated'] = True
    else:
        # Kept proportions aim limbs themselves (keep_proportion_targets).
        targets = {} if keep_height else fallback_targets(ref, names, binds, norm)
        report['calibrated'] = False
        report['warnings'].append('No paired calibration: generic anatomical fitting needs visual review.')
    keep = None
    if keep_height:
        # Keep the source's own shape at this standing height; the engine plays
        # stock rotations on its bone lengths (manifest "proportions").
        factor = keep_height/np.ptp(mesh_bounds(ref, True)[:, 1])
        norm[:3, :] *= factor
        scale *= factor
        keep_fits, keep = keep_proportion_targets(ref, target_names, names, binds, norm)
        stock = {n: ref.worlds[j] for n, j in zip(target_names, ref.doc['skins'][0]['joints'])}
        foot = lambda rig: (rig['LEFTFOOT'][1, 3]+rig['RIGHTFOOT'][1, 3])/2
        leg = (keep['HIPS'][1, 3]-foot(keep))/(stock['HIPS'][1, 3]-foot(stock))
        # leg_ratio: hips-to-ankle height against the stock rig's; ankle: the
        # character's ankle joint height over its soles (the engine keeps the
        # soles where the stock skater's are).
        report.update(mode='keep_source_proportions', scale=float(scale),
                      hips_ratio=float(keep['HIPS'][1, 3]/stock['HIPS'][1, 3]),
                      leg_ratio=float(leg), ankle=float(foot(keep)), stock_ankle=float(foot(stock)))
        for i, n in enumerate(target_names):
            inverse[i] = np.linalg.inv(keep[n])
    normalized = {}
    for i, m in binds.items():
        m = norm@m
        lengths = np.linalg.norm(m[:3,:3],axis=0)
        require((lengths>1e-10).all(), 'Degenerate bind axes')
        m[:3,:3] /= lengths
        require(np.allclose(m[:3,:3].T@m[:3,:3],np.eye(3),atol=1e-3) and
                np.linalg.det(m[:3,:3])>0, 'Sheared or mirrored bind rig is unsupported')
        normalized[i] = m
    target_index = {name:i for i,name in enumerate(target_names)}
    def mapped(i):
        original = i
        while key(doc.doc['nodes'][i].get('name','')) not in MAP:
            require(i in doc.parents, 'Weighted bone cannot map to stock: '+doc.doc['nodes'][original].get('name',''))
            i = doc.parents[i]
        k = key(doc.doc['nodes'][i]['name'])
        if original != i:
            report['collapsed_bones'][doc.doc['nodes'][original]['name']] = MAP[k]
        return i, k, target_index[MAP[k]]
    # Jiggle joints (keep mode): extra skin joints after the stock ones, each
    # under its nearest jiggle ancestor or else its mapped stock joint.
    jiggles = []
    if keep:
        weighted = sorted({j for node in doc.doc['nodes'] if 'mesh' in node for j in doc.doc['skins'][node['skin']]['joints']})
        for j in weighted:
            name = jiggle_name(doc.doc['nodes'][j].get('name', ''))
            if name and key(doc.doc['nodes'][j].get('name', '')) not in MAP:
                jiggles.append(j)
        # Parents before children.
        depth = lambda j: 0 if j not in doc.parents else 1 + depth(doc.parents[j])
        jiggles.sort(key=depth)
    jiggle_index = {j: len(target_names) + n for n, j in enumerate(jiggles)}
    jiggle_world = {}
    jiggle_parent = {}
    for j in jiggles:
        ancestor, k, _ = mapped(j)
        fit = keep_fits.get(k)
        jiggle_world[j] = fit @ np.linalg.inv(normalized[ancestor]) @ normalized[j]
        parent = doc.parents.get(j)
        while parent is not None and parent not in jiggle_index and key(doc.doc['nodes'][parent].get('name', '')) not in MAP:
            parent = doc.parents.get(parent)
        jiggle_parent[j] = parent if parent in jiggle_index else MAP[k]
    report['jiggle_bones'] = [jiggle_name(doc.doc['nodes'][j]['name']) for j in jiggles]
    writer = Writer()
    writer.materials_from(doc)
    primitives = []
    for ni,node in enumerate(doc.doc['nodes']):
        if 'mesh' not in node:
            continue
        skin = doc.doc['skins'][node['skin']]
        matrices, mapped_joints = [], []
        for joint in skin['joints']:
            ancestor,k,ti = mapped(joint)
            # Unavailable destination fingers/endpoints follow the mapped ancestor.
            use = joint if key(doc.doc['nodes'][joint].get('name','')) in targets else ancestor
            tk = key(doc.doc['nodes'][use].get('name',''))
            if keep:
                # Unmapped source joints ride along with their mapped ancestor.
                fit = keep_fits.get(tk, keep_fits.get(k))
                matrices.append(fit @ np.linalg.inv(normalized[use if tk in keep_fits else ancestor]) @ norm @ doc.worlds[ni])
            else:
                matrices.append(targets[tk] @ np.linalg.inv(normalized[use]) @ norm @ doc.worlds[ni])
            mapped_joints.append(jiggle_index.get(joint, ti))
        matrices = np.asarray(matrices)
        require(np.isfinite(matrices).all(), 'Invalid fit transforms')
        for p in doc.doc['meshes'][node['mesh']]['primitives']:
            require(p.get('mode',4) == 4, 'Only triangle meshes are supported')
            require(not p.get('targets'), 'Blendshapes are not supported by version 1; export the neutral mesh')
            attrs = p['attributes']
            require('NORMAL' in attrs, 'Normals are missing')
            positions, normals = doc.accessor(attrs['POSITION']), doc.accessor(attrs['NORMAL'])
            joints, weights = source_skin(doc,p,skin)
            require(len(positions)==len(normals)==len(joints), 'Attribute lengths differ')
            require(report['vertices']+len(positions) <= 1_000_000, 'Character exceeds 1 million vertices')
            blended = np.einsum('vi,vijk->vjk', weights, matrices[joints])
            require((np.linalg.det(blended[:,:3,:3]) > 1e-10).all(),
                    'Fit folds or collapses the mesh; export a neutral T-pose')
            fitted = np.einsum('vij,vj->vi',blended[:,:3,:3],positions)+blended[:,:3,3]
            normal_fit = np.einsum('vij,vj->vi',np.linalg.inv(blended[:,:3,:3]).transpose(0,2,1),normals)
            normal_fit /= np.maximum(np.linalg.norm(normal_fit,axis=1,keepdims=True),1e-12)
            merged = np.zeros((len(positions),len(target_names)+len(jiggles)))
            for column in range(joints.shape[1]):
                np.add.at(merged,(np.arange(len(positions)),np.asarray(mapped_joints)[joints[:,column]]),weights[:,column])
            output_joints = np.argsort(-merged,axis=1,kind='stable')[:,:4]
            output_weights = np.take_along_axis(merged,output_joints,axis=1)
            discarded = 1-output_weights.sum(axis=1)
            report['max_discarded_weight'] = max(report['max_discarded_weight'],float(discarded.max()))
            require(discarded.max() <= 0.1, 'Reducing to four weights loses over 10%; simplify skin weights first')
            output_weights /= output_weights.sum(axis=1,keepdims=True)
            output_joints[output_weights==0] = 0
            out_attrs = {'POSITION':writer.accessor(fitted,'VEC3'), 'NORMAL':writer.accessor(normal_fit,'VEC3'),
                         'JOINTS_0':writer.accessor(output_joints,'VEC4',5123),
                         'WEIGHTS_0':writer.accessor(output_weights,'VEC4')}
            for name in ('TEXCOORD_0','TEXCOORD_1','COLOR_0'):
                if name in attrs:
                    values = doc.accessor(attrs[name])
                    require(len(values)==len(positions), 'Attribute length differs: '+name)
                    out_attrs[name] = writer.accessor(values,'VEC'+str(values.shape[1]))
            indices = doc.accessor(p['indices']).ravel() if 'indices' in p else np.arange(len(positions))
            require(len(indices)%3==0 and indices.min()>=0 and indices.max()<len(positions),'Invalid triangle indices')
            out = {'attributes':out_attrs,'indices':writer.accessor(indices,'SCALAR',5125)}
            if 'material' in p:
                out['material'] = p['material']
            primitives.append(out)
            report['vertices'] += len(positions)
            report['triangles'] += len(indices)//3
    if include_board:
        offset = writer.materials_from(ref)
        for mesh in ref.doc['meshes']:
            for p in mesh['primitives']:
                attrs = p['attributes']
                js = ref.accessor(attrs['JOINTS_0']).astype(int)
                ws = ref.accessor(attrs['WEIGHTS_0'])
                used = set(js[ws>1e-6].ravel())
                if used and all(target_names[j] in BOARD for j in used):
                    out_attrs = {}
                    for name in ('POSITION','NORMAL','TEXCOORD_0','TEXCOORD_1','JOINTS_0','WEIGHTS_0'):
                        if name in attrs:
                            a = ref.doc['accessors'][attrs[name]]
                            values = ref.accessor(attrs[name])
                            if name=='JOINTS_0':
                                values[ws==0] = 0
                            out_attrs[name] = writer.accessor(values,a['type'],5123 if name=='JOINTS_0' else 5126)
                    primitives.append({'attributes':out_attrs,'indices':writer.accessor(ref.accessor(p['indices']).ravel(),'SCALAR',5125),
                                       'material':p['material']+offset})
    # A single shared rig/skin also works with the older binder which binds one skin.
    nodes = []
    node_map = {}
    joint_name = {j: target_names[k] for k, j in enumerate(ref_skin['joints'])}
    for i,n in enumerate(ref.doc['nodes']):
        if 'mesh' not in n:
            node_map[i] = len(nodes)
            transform = clean_node_transform(n)
            if keep and i in joint_name:
                parent = ref.parents.get(i)
                parent_world = keep[joint_name[parent]] if parent in joint_name else (ref.worlds[parent] if parent is not None else np.eye(4))
                local = np.linalg.inv(parent_world) @ keep[joint_name[i]]
                transform = {'matrix': local.T.reshape(-1).tolist()}
            nodes.append({'name':n.get('name',''),**transform})
    for i,oi in node_map.items():
        children = [node_map[c] for c in ref.doc['nodes'][i].get('children',[]) if c in node_map]
        if children:
            nodes[oi]['children'] = children
    roots = [node_map[i] for i in node_map if i not in ref.parents]
    # Jiggle joints hang off the output rig (JIGGLE_<source name>).
    stock_node = {name: node_map[i] for i, name in joint_name.items()}
    jiggle_node = {}
    for j in jiggles:
        parent = jiggle_parent[j]
        parent_world = jiggle_world[parent] if parent in jiggle_world else keep[parent]
        local = np.linalg.inv(parent_world) @ jiggle_world[j]
        jiggle_node[j] = len(nodes)
        nodes.append({'name': jiggle_name(doc.doc['nodes'][j]['name']), 'matrix': local.T.reshape(-1).tolist()})
        owner = nodes[jiggle_node[parent] if parent in jiggle_node else stock_node[parent]]
        owner['children'] = owner.get('children', []) + [jiggle_node[j]]
    if jiggles:
        inverse = np.concatenate([inverse, np.stack([np.linalg.inv(jiggle_world[j]) for j in jiggles])])
    mesh_node = len(nodes)
    nodes.append({'name':'Imported_Skater','mesh':0,'skin':0})
    roots.append(mesh_node)
    writer.doc.update(nodes=nodes,meshes=[{'name':'Converted_Character','primitives':primitives}],
                      skins=[{'joints':[node_map[j] for j in ref_skin['joints']] + [jiggle_node[j] for j in jiggles],
                              'inverseBindMatrices':writer.accessor(inverse.transpose(0,2,1).reshape(-1,16),'MAT4')}],
                      scenes=[{'nodes':roots}],scene=0)
    report['board_included'] = include_board
    report['warnings'].append('Fits stock proportions; fingers collapse to hands. Contact/deformation quality needs in-game review.')
    if include_board:
        report['warnings'].append('Stock board normal maps use engine-generated tangent space, as in the original asset.')
    if doc.doc.get('animations'):
        report['warnings'].append('Source animations omitted; the engine supplies stock skating poses.')
    writer.save(output)
    validate_output(output,reference,keep is not None)
    return report


def validate_output(path, reference, keep=False):
    doc, ref = Document(path), Document(reference)
    _, names, inverse = reference_rig(doc)
    _, expected, ref_inverse = reference_rig(ref)
    # Kept proportions move the body joints; the names and board rig still
    # match. Jiggle joints may follow the stock ones.
    extra = names[len(expected):]
    require(names[:len(expected)]==expected and all(n.startswith('JIGGLE_') for n in extra)
            and (keep or (not extra and np.allclose(inverse,ref_inverse,atol=1e-7))), 'Output rig differs from stock')
    require('animations' not in doc.doc, 'Unexpected imported animation')
    for mesh in doc.doc['meshes']:
        for p in mesh['primitives']:
            positions = doc.accessor(p['attributes']['POSITION'])
            joints, weights = source_skin(doc,p,doc.doc['skins'][0])
            require(len(joints)==len(positions) and joints.shape[1]==4,'Invalid output skin')
            require(np.allclose(doc.accessor(p['attributes']['WEIGHTS_0']).sum(axis=1),1,atol=1e-5),'Weights not normalized')
            indices = doc.accessor(p['indices'])
            require(indices.max()<len(positions),'Invalid output index')
    for i in range(len(doc.doc.get('images',[]))):
        doc.image(i)


def convert_file(source, reference, destination, fbx_tool, profile=None, include_board=True, keep_source=False):
    source,reference,destination = Path(source).resolve(),Path(reference).resolve(),Path(destination).resolve()
    require(source.suffix.lower() in ('.fbx','.glb'), 'Choose a Mixamo FBX or GLB')
    require(source.stat().st_size <= 512*1024**2,'Source exceeds 512 MiB')
    require(not destination.exists(), 'Output folder already exists; choose another name')
    require(source != reference, 'Source and stock reference must be different files')
    destination.parent.mkdir(parents=True,exist_ok=True)
    # Publish the complete directory only after every conversion/validation succeeds.
    with tempfile.TemporaryDirectory(prefix='.mixamo-',dir=destination.parent) as temporary:
        temp = Path(temporary)
        normalized = source
        log = ''
        if source.suffix.lower()=='.fbx':
            require(Path(fbx_tool).is_file(),'FBX2glTF.exe is missing; run the converter setup')
            normalized = temp/'source.glb'
            proc = subprocess.run([str(fbx_tool),'--binary','--input',str(source),'--output',str(temp/'source'),
                                   '--fbx-temp-dir',str(temp),'--compute-normals','missing','--pbr-metallic-roughness'],
                                  capture_output=True,text=True,timeout=180,
                                  creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
            log = proc.stdout+'\n'+proc.stderr
            require(proc.returncode==0 and normalized.is_file(),'FBX conversion failed: '+log[-2000:])
        publish = temp/'publish'
        publish.mkdir()
        report = convert_glb(normalized,reference,publish/'character.glb',profile,include_board)
        if keep_source:
            import shutil
            shutil.copy2(normalized,publish/'source.glb')
        report['original_source_sha256'] = sha(source)
        report['fbx_conversion_log'] = log
        report['output_sha256'] = sha(publish/'character.glb')
        (publish/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
        (publish/'README.txt').write_text('character.glb is a stock-rig appearance asset.\n'
            'Original source and installed assets were not modified.\n'
            'This converter does not install the character or add a menu importer.\n'
            'Inspect report.json. Gameplay/visual validation is still required.\n',encoding='utf-8')
        publish.rename(destination)
    return destination/'character.glb'
