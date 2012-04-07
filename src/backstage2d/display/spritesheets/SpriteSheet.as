package backstage2d.display.spritesheets
{
    import flash.display.BitmapData;
	import flash.system.System;
	import flash.utils.Dictionary;
	
    import flash.geom.Point;
    import flash.geom.Rectangle;
    import flash.geom.Matrix;
	
	import backstage2d.display.Actor;
	
	import backstage2d.namespaces.bs2d;
	
	public class SpriteSheet
	{
	use namespace bs2d;
		public var ss:BitmapData// = null;
		bs2d var nodes:Vector.<SpriteNode> = new <SpriteNode>[];
		
		protected var tree:SpriteNode = new SpriteNode();
		
		public var complete:Boolean = false;
		
		/**
		 * 
		 * @param	width
		 * @param	height
		 */
		public function SpriteSheet( width:int, height:int ):void
		{
			ss = new BitmapData( width, height, true, 0x00FFFFFF );
			tree.rect = new Rectangle( 0, 0, width, height );
		}
		
		// :TODO: Add a scale matrix argument. Can be used in cases where
		// you'd want to conserve memory.
		/**
		 * Makes a Sprite Sheet out of alist of actors.
		 * @param	actors The list actors to create a Spritesheet out of.
		 * @return
		 */
		public function make( actors:Vector.<Actor> ):BitmapData
		{
			actors = actors.concat().sort( sortByBigPerimeter );
			
			var scratchBMD:BitmapData = new BitmapData( ss.width, ss.height, true, 0x00000000 );
			var fillRect:Rectangle = new Rectangle(0, 0, ss.width, ss.height);
			
			var m:Matrix = new Matrix;
			var p:Point = new Point;
			
			var bounds:Rectangle;
			var leaf:SpriteNode;
			scratchBMD.lock();
			ss.lock();
			for (var i:int = 0, n:int = actors.length, actor:Actor; i < n; ++i )
			{
				scratchBMD.fillRect( fillRect, 0x00000000 );
				actor = actors[i];
				
				// This doesn't take into account filters, and other oddities.
				// It's really to just roughly get it the DO in the middleish.
				m.identity();
				m.tx = ss.width/2 - (actor.width / 2);
				m.ty = ss.height/2 - (actor.height / 2);
				
				// :TODO: Make it so we can adjust quality per Actor.
				//stage.quality = "16X16LINEAR";
				actor.draw( scratchBMD, m );
				
				bounds = scratchBMD.getColorBoundsRect(0xFFFFFFFF, 0x000000, false);
				
				// find new center point and offset
				
				
				if ( actor.texNode )
					leaf = actor.texNode;
				else
					leaf = tree.insert( bounds );
				
				if ( !leaf ) {
					// :TODO Should throw an error here - the user can respond by allocating
					// a larger bitmapdata, or setting a rection scale value.
					trace( 'image wouldn\'t fit :TODO: throw an error');
				}
				else {
					actor.texID = nodes.length;
					nodes.push( leaf );
					
					// Allows bitmapData reuse if upstream element uses duplicate content
					actor.texNode = leaf;
					
					//leaf.actor = actor;
					p.x = leaf.rect.x;
					p.y = leaf.rect.y;
					ss.copyPixels( scratchBMD, bounds, p );
				}
			}
			complete = true;
			scratchBMD.dispose();
			ss.unlock();
			//System.gc();
			return ss;
		}
        
		private function sortByBigPerimeter( a:Actor, b:Actor ):int
		{
			var asize:int = a.width + a.height;
			var bsize:int = b.width + b.height;
			
			if (asize > bsize) {
				return -1;
			}
			else if (asize < bsize) {
				return 1;
			}
			else {
				return 0;
			}
		}
		
    }
}
